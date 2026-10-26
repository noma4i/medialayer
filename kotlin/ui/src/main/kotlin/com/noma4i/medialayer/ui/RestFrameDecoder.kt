package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Movie
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Rect
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DecodeResult
import coil3.decode.DecodeUtils
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.request.Options
import coil3.size.Dimension
import com.noma4i.medialayer.decodeCost
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicLong
import kotlin.coroutines.coroutineContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Decodes the last frame of an animated WebP or GIF as a still; any other source goes on to the next decoder. */
internal class RestFrameDecoder(private val result: SourceFetchResult, private val options: Options, private val admit: DecodeAdmit) : Decoder {
  override suspend fun decode(): DecodeResult? {
    val bytes = result.source.source().peek().readByteArray()
    val width = (options.size.width as? Dimension.Pixels)?.px
    val height = (options.size.height as? Dimension.Pixels)?.px
    val plan = restFramePlan(bytes, width, height, options.scale) ?: return null
    val hold = admit(plan.bytes)
    try {
      val check = coroutineContext
      val last = withContext(Dispatchers.Default) { lastFrame(bytes, plan.multiplier) { check.ensureActive() } } ?: return null
      return DecodeResult(last.asImage(), true)
    } finally {
      hold.close()
    }
  }

  class Factory(private val admit: DecodeAdmit) : Decoder.Factory {
    override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder = RestFrameDecoder(result, options, admit)
  }
}

internal class RestFramePlan(val multiplier: Double, val bytes: Long)

internal fun restFramePlan(bytes: ByteArray, width: Int?, height: Int?, scale: coil3.size.Scale): RestFramePlan? {
  val format = animatedFormat(bytes) ?: return null
  val canvas = when (format) {
    AnimatedFormat.Webp -> WebpAnimation.parse(bytes).let { it.width to it.height }
    AnimatedFormat.Gif -> if (bytes.size < 10) return null else ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).let { (it.getShort(6).toInt() and 0xFFFF) to (it.getShort(8).toInt() and 0xFFFF) }
  }
  val full = decodeCost(canvas.first.toUInt(), canvas.second.toUInt()) ?: throw IllegalArgumentException("the animation is above the pixel limit")
  val multiplier = if (width == null || height == null) 1.0 else minOf(1.0, DecodeUtils.computeSizeMultiplier(canvas.first, canvas.second, width, height, scale))
  val shown = scaledSide(canvas.first, multiplier).toLong() * scaledSide(canvas.second, multiplier) * 4
  val working = when (format) {
    AnimatedFormat.Webp -> WebpAnimation.parse(bytes).frames.maxOfOrNull { it.width.toLong() * it.height * 4 + it.chunkLength } ?: 0L
    AnimatedFormat.Gif -> full.toLong()
  }
  return RestFramePlan(multiplier, shown + working + bytes.size)
}

private fun scaledSide(side: Int, multiplier: Double): Int = (side * multiplier).toInt().coerceAtLeast(1)

internal enum class AnimatedFormat { Webp, Gif }

internal fun animatedFormat(bytes: ByteArray): AnimatedFormat? = when {
  WebpAnimation.isAnimated(bytes) -> AnimatedFormat.Webp
  bytes.size > 6 && String(bytes, 0, 4, Charsets.ISO_8859_1) == "GIF8" -> AnimatedFormat.Gif
  else -> null
}

internal fun lastFrame(bytes: ByteArray, multiplier: Double = 1.0, check: () -> Unit = {}): Bitmap? = when (animatedFormat(bytes)) {
  AnimatedFormat.Webp -> WebpAnimation.parse(bytes).lastFrame(multiplier, check)
  AnimatedFormat.Gif -> gifLastFrame(bytes, multiplier, check)
  null -> null
}

private fun gifLastFrame(bytes: ByteArray, multiplier: Double, check: () -> Unit): Bitmap? {
  @Suppress("DEPRECATION")
  val movie = Movie.decodeByteArray(bytes, 0, bytes.size) ?: return null
  if (movie.width() <= 0 || movie.height() <= 0) return null
  check()
  movie.setTime(maxOf(movie.duration() - 1, 0))
  val bitmap = Bitmap.createBitmap(scaledSide(movie.width(), multiplier), scaledSide(movie.height(), multiplier), Bitmap.Config.ARGB_8888)
  check()
  Canvas(bitmap).apply { scale(multiplier.toFloat(), multiplier.toFloat()) }.let { movie.draw(it, 0f, 0f) }
  return bitmap
}

internal interface FrameSurface {
  fun clear(x: Int, y: Int, width: Int, height: Int)
  fun draw(still: ByteArray, x: Int, y: Int, blends: Boolean)
}

private class BitmapSurface(width: Int, height: Int, multiplier: Double) : FrameSurface {
  val bitmap: Bitmap = Bitmap.createBitmap(scaledSide(width, multiplier), scaledSide(height, multiplier), Bitmap.Config.ARGB_8888)
  private val canvas = Canvas(bitmap).apply { scale(multiplier.toFloat(), multiplier.toFloat()) }
  private val overwrite = Paint().apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.SRC) }
  private val erase = Paint().apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.CLEAR) }

  override fun clear(x: Int, y: Int, width: Int, height: Int) = canvas.drawRect(Rect(x, y, x + width, y + height), erase)

  override fun draw(still: ByteArray, x: Int, y: Int, blends: Boolean) {
    val piece = requireNotNull(BitmapFactory.decodeByteArray(still, 0, still.size)) { "WebP frame does not decode" }
    canvas.drawBitmap(piece, x.toFloat(), y.toFloat(), if (blends) null else overwrite)
    piece.recycle()
  }
}

internal class WebpFrame(
  val x: Int,
  val y: Int,
  val width: Int,
  val height: Int,
  val durationMs: Int,
  val blends: Boolean,
  val disposesToBackground: Boolean,
  private val source: ByteArray,
  private val chunkStart: Int,
  val chunkLength: Int
) {
  fun stillFile(): ByteArray {
    val total = STILL_HEADER + chunkLength
    DecodeCopies.bytes.addAndGet(total.toLong())
    return ByteBuffer.allocate(total).order(ByteOrder.LITTLE_ENDIAN)
      .putInt(FOURCC_RIFF).putInt(total - 8).putInt(FOURCC_WEBP)
      .putInt(FOURCC_VP8X).putInt(10).putInt(ALPHA).put24(width - 1).put24(height - 1)
      .put(source, chunkStart, chunkLength).array()
  }
}

internal object DecodeCopies {
  val bytes = AtomicLong()
}

internal class WebpAnimation(val width: Int, val height: Int, val frames: List<WebpFrame>) {
  val durationMs: Int get() = frames.sumOf { it.durationMs }

  /** Lays the frames one over the other: the dispose of the previous frame first, then the still file of the frame. */
  fun compose(surface: FrameSurface, check: () -> Unit = {}) {
    frames.forEachIndexed { index, frame ->
      check()
      frames.getOrNull(index - 1)?.takeIf { it.disposesToBackground }?.let { surface.clear(it.x, it.y, it.width, it.height) }
      surface.draw(frame.stillFile(), frame.x, frame.y, frame.blends)
    }
  }

  fun lastFrame(multiplier: Double = 1.0, check: () -> Unit = {}): Bitmap = BitmapSurface(width, height, multiplier).also { compose(it, check) }.bitmap

  companion object {
    fun isAnimated(bytes: ByteArray): Boolean = bytes.size > 12 && parseHeader(bytes)?.let { buffer -> chunkTypes(bytes, buffer).contains(FOURCC_ANMF) } == true

    fun parse(bytes: ByteArray): WebpAnimation {
      val buffer = requireNotNull(parseHeader(bytes)) { "not a WebP file" }
      var width = 0
      var height = 0
      val frames = mutableListOf<WebpFrame>()
      var position = 12
      while (position + 8 <= bytes.size) {
        val type = buffer.getInt(position)
        val size = buffer.getInt(position + 4)
        val data = position + 8
        if (type == FOURCC_VP8X) {
          width = buffer.get24(data + 4) + 1
          height = buffer.get24(data + 7) + 1
        }
        if (type == FOURCC_ANMF) {
          val flags = buffer.get(data + 15).toInt()
          frames += WebpFrame(
            x = buffer.get24(data) * 2,
            y = buffer.get24(data + 3) * 2,
            width = buffer.get24(data + 6) + 1,
            height = buffer.get24(data + 9) + 1,
            durationMs = buffer.get24(data + 12),
            blends = flags and 2 == 0,
            disposesToBackground = flags and 1 == 1,
            source = bytes,
            chunkStart = data + ANMF_HEADER,
            chunkLength = size - ANMF_HEADER
          )
        }
        position = data + size + (size and 1)
      }
      return WebpAnimation(width, height, frames)
    }

    private fun parseHeader(bytes: ByteArray): ByteBuffer? {
      if (bytes.size < 12) return null
      val buffer = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
      return buffer.takeIf { it.getInt(0) == FOURCC_RIFF && it.getInt(8) == FOURCC_WEBP }
    }

    private fun chunkTypes(bytes: ByteArray, buffer: ByteBuffer): List<Int> {
      val types = mutableListOf<Int>()
      var position = 12
      while (position + 8 <= bytes.size) {
        types += buffer.getInt(position)
        position += 8 + buffer.getInt(position + 4) + (buffer.getInt(position + 4) and 1)
      }
      return types
    }
  }
}

private const val FOURCC_RIFF = 0x46464952
private const val FOURCC_WEBP = 0x50424557
private const val FOURCC_VP8X = 0x58385056
private const val FOURCC_ANMF = 0x464D4E41
private const val ANMF_HEADER = 16
private const val STILL_HEADER = 30
private const val ALPHA = 0x10

private fun ByteBuffer.put24(value: Int): ByteBuffer = put(value.toByte()).put((value shr 8).toByte()).put((value shr 16).toByte())

private fun ByteBuffer.get24(index: Int) = (get(index).toInt() and 0xFF) or ((get(index + 1).toInt() and 0xFF) shl 8) or ((get(index + 2).toInt() and 0xFF) shl 16)

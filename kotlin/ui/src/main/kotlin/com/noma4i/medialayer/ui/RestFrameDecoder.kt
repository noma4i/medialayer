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
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Decodes the last frame of an animated WebP or GIF as a still; any other source goes on to the next decoder. */
internal class RestFrameDecoder(private val result: SourceFetchResult, private val options: Options) : Decoder {
  override suspend fun decode(): DecodeResult? {
    val bytes = result.source.source().peek().readByteArray()
    val last = withContext(Dispatchers.Default) { lastFrame(bytes) } ?: return null
    val width = (options.size.width as? Dimension.Pixels)?.px
    val height = (options.size.height as? Dimension.Pixels)?.px
    val shown = if (width == null || height == null) last else {
      val multiplier = DecodeUtils.computeSizeMultiplier(last.width, last.height, width, height, options.scale)
      if (multiplier >= 1.0) last else Bitmap.createScaledBitmap(last, (last.width * multiplier).toInt().coerceAtLeast(1), (last.height * multiplier).toInt().coerceAtLeast(1), true)
    }
    return DecodeResult(shown.asImage(), true)
  }

  class Factory : Decoder.Factory {
    override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder = RestFrameDecoder(result, options)
  }
}

internal enum class AnimatedFormat { Webp, Gif }

internal fun animatedFormat(bytes: ByteArray): AnimatedFormat? = when {
  WebpAnimation.isAnimated(bytes) -> AnimatedFormat.Webp
  bytes.size > 6 && String(bytes, 0, 4, Charsets.ISO_8859_1) == "GIF8" -> AnimatedFormat.Gif
  else -> null
}

internal fun lastFrame(bytes: ByteArray): Bitmap? = when (animatedFormat(bytes)) {
  AnimatedFormat.Webp -> WebpAnimation.parse(bytes).lastFrame()
  AnimatedFormat.Gif -> gifLastFrame(bytes)
  null -> null
}

private fun gifLastFrame(bytes: ByteArray): Bitmap? {
  @Suppress("DEPRECATION")
  val movie = Movie.decodeByteArray(bytes, 0, bytes.size) ?: return null
  if (movie.width() <= 0 || movie.height() <= 0) return null
  movie.setTime(maxOf(movie.duration() - 1, 0))
  val bitmap = Bitmap.createBitmap(movie.width(), movie.height(), Bitmap.Config.ARGB_8888)
  movie.draw(Canvas(bitmap), 0f, 0f)
  return bitmap
}

internal interface FrameSurface {
  fun clear(x: Int, y: Int, width: Int, height: Int)
  fun draw(still: ByteArray, x: Int, y: Int, blends: Boolean)
}

private class BitmapSurface(width: Int, height: Int) : FrameSurface {
  val bitmap: Bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
  private val canvas = Canvas(bitmap)
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
  val chunks: ByteArray
) {
  fun stillFile(): ByteArray {
    val header = ByteBuffer.allocate(18).order(ByteOrder.LITTLE_ENDIAN)
      .putInt(FOURCC_VP8X).putInt(10).putInt(ALPHA).put24(width - 1).put24(height - 1)
    val body = ByteBuffer.allocate(4 + header.capacity() + chunks.size).order(ByteOrder.LITTLE_ENDIAN)
      .putInt(FOURCC_WEBP).put(header.array()).put(chunks)
    return ByteBuffer.allocate(8 + body.capacity()).order(ByteOrder.LITTLE_ENDIAN)
      .putInt(FOURCC_RIFF).putInt(body.capacity()).put(body.array()).array()
  }
}

internal class WebpAnimation(val width: Int, val height: Int, val frames: List<WebpFrame>) {
  val durationMs: Int get() = frames.sumOf { it.durationMs }

  /** Lays the frames one over the other: the dispose of the previous frame first, then the still file of the frame. */
  fun compose(surface: FrameSurface) {
    frames.forEachIndexed { index, frame ->
      frames.getOrNull(index - 1)?.takeIf { it.disposesToBackground }?.let { surface.clear(it.x, it.y, it.width, it.height) }
      surface.draw(frame.stillFile(), frame.x, frame.y, frame.blends)
    }
  }

  fun lastFrame(): Bitmap = BitmapSurface(width, height).also(::compose).bitmap

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
            chunks = bytes.copyOfRange(data + ANMF_HEADER, data + size)
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
private const val ALPHA = 0x10

private fun ByteBuffer.put24(value: Int): ByteBuffer = put(value.toByte()).put((value shr 8).toByte()).put((value shr 16).toByte())

private fun ByteBuffer.get24(index: Int) = (get(index).toInt() and 0xFF) or ((get(index + 1).toInt() and 0xFF) shl 8) or ((get(index + 2).toInt() and 0xFF) shl 16)

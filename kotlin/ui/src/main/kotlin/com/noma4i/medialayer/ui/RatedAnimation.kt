@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorFilter
import android.graphics.Movie
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import android.os.SystemClock
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.request.Options
import coil3.size.Dimension
import java.util.concurrent.Executor
import kotlin.coroutines.coroutineContext
import kotlin.math.ceil
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asExecutor
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

internal fun playbackRateOf(rate: Double): Double = if (rate > 0 && rate.isFinite()) rate else 1.0

internal fun playsAtRate(rate: Double, resting: RestFrame?): Boolean = resting == null && playbackRateOf(rate) != 1.0

internal fun frameDelayMs(delayMs: Int): Int = if (delayMs < MINIMUM_DELAY_MS) DEFAULT_DELAY_MS else delayMs

private const val MINIMUM_DELAY_MS = 20
private const val DEFAULT_DELAY_MS = 100

internal class AnimationTimeline(delaysMs: IntArray) {
  val frameCount = delaysMs.size
  val frameStarts = IntArray(frameCount)
  val frameEnds = LongArray(frameCount)
  val durationMs: Long

  init {
    var sum = 0L
    delaysMs.forEachIndexed { index, delay ->
      frameStarts[index] = sum.toInt()
      sum += delay
      frameEnds[index] = sum
    }
    durationMs = sum
  }

  fun tickAt(positionMs: Double): Long {
    val position = maxOf(0.0, positionMs)
    val round = (position / durationMs).toLong()
    val offset = position - round * durationMs
    var low = 0
    var high = frameCount
    while (low < high) {
      val middle = (low + high) / 2
      if (offset < frameEnds[middle]) high = middle else low = middle + 1
    }
    return round * frameCount + minOf(low, frameCount - 1)
  }

  fun endOf(tick: Long): Double = (tick / frameCount) * durationMs.toDouble() + frameEnds[(tick % frameCount).toInt()]

  fun middleOf(index: Int): Int = frameStarts[index] + ((frameEnds[index] - frameStarts[index]) / 2).toInt()
}

internal fun interface FramePainter {
  fun paint(index: Int, into: Bitmap)
}

internal class RatedAnimation(
  private val timeline: AnimationTimeline,
  private val painter: FramePainter,
  first: Bitmap,
  spare: Bitmap,
  rate: Double,
  private val repeats: Repeats,
  private val clock: () -> Long = SystemClock::uptimeMillis,
  private val executor: Executor = Dispatchers.Default.asExecutor()
) : Drawable(), RepeatingAnimation {
  var rate = playbackRateOf(rate)
    private set
  var shownFrame = 0
    private set
  private var shown = first
  private var spare = spare
  private var rendering = false
  private var running = false
  private var finished = false
  private var anchorAt = 0L
  private var anchorPosition = 0.0
  private var onFinished: (() -> Unit)? = null
  private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
  private val step = Runnable { advance() }

  override fun onFinished(callback: () -> Unit) {
    onFinished = callback
  }

  fun rateTo(value: Double) {
    val next = playbackRateOf(value)
    if (next == rate) return
    if (running) {
      val now = clock()
      anchorPosition = positionAt(now)
      anchorAt = now
    }
    rate = next
    if (running) advance()
  }

  override fun start() {
    if (running) return
    if (finished) {
      finished = false
      anchorPosition = 0.0
    }
    running = true
    anchorAt = clock()
    advance()
  }

  override fun stop() {
    if (!running) return
    anchorPosition = positionAt(clock())
    running = false
    unscheduleSelf(step)
  }

  override fun isRunning() = running

  private fun positionAt(now: Long): Double = if (running) anchorPosition + (now - anchorAt) * rate else anchorPosition

  private fun advance() {
    if (!running) return
    unscheduleSelf(step)
    val now = clock()
    val position = positionAt(now)
    val plays = (repeats as? Repeats.Times)?.let { maxOf(it.count, 1) }
    if (plays != null && position >= plays * timeline.durationMs) {
      finish()
      return
    }
    val tick = timeline.tickAt(position)
    show((tick % timeline.frameCount).toInt())
    scheduleSelf(step, now + ceil((timeline.endOf(tick) - position) / rate).toLong().coerceAtLeast(1))
  }

  private fun finish() {
    running = false
    finished = true
    show(timeline.frameCount - 1)
    onFinished?.invoke()
  }

  private fun show(index: Int) {
    if (index == shownFrame || rendering) return
    rendering = true
    val target = spare
    executor.execute {
      painter.paint(index, target)
      scheduleSelf(Runnable { present(target, index) }, clock())
    }
  }

  private fun present(target: Bitmap, index: Int) {
    spare = shown
    shown = target
    shownFrame = index
    rendering = false
    invalidateSelf()
    if (running) advance() else if (finished) show(timeline.frameCount - 1)
  }

  override fun draw(canvas: Canvas) {
    canvas.drawBitmap(shown, null, bounds, paint)
  }

  override fun setAlpha(alpha: Int) {
    paint.alpha = alpha
  }

  override fun setColorFilter(colorFilter: ColorFilter?) {
    paint.colorFilter = colorFilter
  }

  @Deprecated("Deprecated in Java")
  override fun getOpacity() = PixelFormat.TRANSLUCENT

  override fun getIntrinsicWidth() = shown.width

  override fun getIntrinsicHeight() = shown.height
}

private class MoviePainter(private val movie: Movie, private val timeline: AnimationTimeline, private val multiplier: Double) : FramePainter {
  override fun paint(index: Int, into: Bitmap) {
    into.eraseColor(Color.TRANSPARENT)
    movie.setTime(timeline.middleOf(index))
    movie.draw(Canvas(into).apply { scale(multiplier.toFloat(), multiplier.toFloat()) }, 0f, 0f)
  }
}

private class WebpPainter(private val animation: WebpAnimation, private val surface: BitmapSurface) : FramePainter {
  private var composed = -1

  override fun paint(index: Int, into: Bitmap) {
    if (index <= composed) {
      surface.reset()
      composed = -1
    }
    while (composed < index) {
      composed += 1
      animation.composeFrame(surface, composed)
    }
    into.eraseColor(Color.TRANSPARENT)
    Canvas(into).drawBitmap(surface.bitmap, 0f, 0f, null)
  }
}

internal class GifFrames(val delaysMs: IntArray, val bytes: ByteArray)

/** The frame delays of a GIF, and a copy of its bytes where a delay under 20 ms reads 100 ms, so the platform movie follows the same table. */
internal fun gifFrames(source: ByteArray): GifFrames? {
  if (source.size < 14) return null
  val bytes = source.copyOf()
  val delays = ArrayList<Int>()
  var position = 13 + colorTableSize(bytes[10].toInt())
  var delayAt = -1
  while (position < bytes.size) {
    when (bytes[position].toInt() and 0xFF) {
      EXTENSION -> {
        if (position + 6 <= bytes.size && bytes[position + 1].toInt() and 0xFF == GRAPHIC_CONTROL) delayAt = position + 4
        position = skipBlocks(bytes, position + 2)
      }
      IMAGE -> {
        if (position + 10 > bytes.size) break
        var raw = if (delayAt >= 0) (bytes[delayAt].toInt() and 0xFF) or ((bytes[delayAt + 1].toInt() and 0xFF) shl 8) else 0
        if (raw * 10 < MINIMUM_DELAY_MS) {
          raw = DEFAULT_DELAY_MS / 10
          if (delayAt >= 0) {
            bytes[delayAt] = raw.toByte()
            bytes[delayAt + 1] = 0
          }
        }
        delays.add(raw * 10)
        delayAt = -1
        position += 10 + colorTableSize(bytes[position + 9].toInt()) + 1
        position = skipBlocks(bytes, position)
      }
      else -> break
    }
  }
  return if (delays.size < 2) null else GifFrames(delays.toIntArray(), bytes)
}

private const val EXTENSION = 0x21
private const val IMAGE = 0x2C
private const val GRAPHIC_CONTROL = 0xF9

private fun colorTableSize(flags: Int): Int = if (flags and 0x80 != 0) 3 * (1 shl ((flags and 7) + 1)) else 0

private fun skipBlocks(bytes: ByteArray, from: Int): Int {
  var position = from
  while (position < bytes.size) {
    val length = bytes[position].toInt() and 0xFF
    position += 1 + length
    if (length == 0) break
  }
  return position
}

internal fun ratedAnimation(bytes: ByteArray, multiplier: Double, rate: Double, repeats: Repeats, executor: Executor = Dispatchers.Default.asExecutor(), check: () -> Unit = {}): RatedAnimation? {
  val format = animatedFormat(bytes) ?: return null
  val timeline: AnimationTimeline
  val painter: FramePainter
  val width: Int
  val height: Int
  when (format) {
    AnimatedFormat.Webp -> {
      val animation = WebpAnimation.parse(bytes)
      if (animation.frames.size < 2) return null
      timeline = AnimationTimeline(IntArray(animation.frames.size) { frameDelayMs(animation.frames[it].durationMs) })
      val surface = BitmapSurface(animation.width, animation.height, multiplier)
      painter = WebpPainter(animation, surface)
      width = surface.bitmap.width
      height = surface.bitmap.height
    }
    AnimatedFormat.Gif -> {
      val gif = gifFrames(bytes) ?: return null
      val movie = Movie.decodeByteArray(gif.bytes, 0, gif.bytes.size) ?: return null
      if (movie.width() <= 0 || movie.height() <= 0) return null
      timeline = AnimationTimeline(gif.delaysMs)
      painter = MoviePainter(movie, timeline, multiplier)
      width = scaledSide(movie.width(), multiplier)
      height = scaledSide(movie.height(), multiplier)
    }
  }
  check()
  val first = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
  val spare = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
  painter.paint(0, first)
  check()
  return RatedAnimation(timeline, painter, first, spare, rate, repeats, executor = executor)
}

internal class RatedAnimationDecoder(private val result: SourceFetchResult, private val options: Options, private val admit: DecodeAdmit, private val rate: Double, private val repeats: Repeats) : Decoder {
  override suspend fun decode(): DecodeResult? {
    val bytes = result.source.source().peek().readByteArray()
    val width = (options.size.width as? Dimension.Pixels)?.px
    val height = (options.size.height as? Dimension.Pixels)?.px
    val plan = restFramePlan(bytes, width, height, options.scale) ?: return null
    val hold = admit(plan.bytes + 2 * plan.shown + bytes.size)
    try {
      val check = coroutineContext
      val animation = withContext(Dispatchers.Default) { ratedAnimation(bytes, plan.multiplier, rate, repeats) { check.ensureActive() } } ?: return null
      return DecodeResult(animation.asImage(false), true)
    } finally {
      hold.close()
    }
  }

  class Factory(private val admit: DecodeAdmit, private val rate: Double, private val repeats: Repeats) : Decoder.Factory {
    override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder = RatedAnimationDecoder(result, options, admit, rate, repeats)
  }
}

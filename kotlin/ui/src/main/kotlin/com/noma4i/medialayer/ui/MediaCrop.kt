package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.PointF
import android.graphics.RectF
import android.util.Size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.noma4i.medialayer.CropAspect
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.CropGestureReport
import com.noma4i.medialayer.CropGestureTrack
import com.noma4i.medialayer.CropHandle
import com.noma4i.medialayer.CropLayout
import com.noma4i.medialayer.CropLimits
import com.noma4i.medialayer.CropSetup
import com.noma4i.medialayer.CropView
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.cropDragView
import com.noma4i.medialayer.cropFinish
import com.noma4i.medialayer.cropGestureBegin
import com.noma4i.medialayer.cropGestureEnd
import com.noma4i.medialayer.cropGestureMove
import com.noma4i.medialayer.cropHandleAt
import com.noma4i.medialayer.cropInitial
import com.noma4i.medialayer.cropLayout
import com.noma4i.medialayer.cropPanView
import com.noma4i.medialayer.cropTelemetry
import com.noma4i.medialayer.cropTurn
import com.noma4i.medialayer.cropZoomView
import com.noma4i.medialayer.defaultCropLimits
import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.CancellationException

/**
 * What the user chose: [crop] is a rectangle in fractions 0 to 1 of the photo after [rotation] (origin top left),
 * [rotation] is the clockwise turn in degrees, 0, 90, 180 or 270. Give both to `PhotoEdit(crop = ..., rotationDegrees = ...)` in a `mediaPrepareDescriptor` image intent.
 */
data class MediaCropResult(
  val crop: RectF,
  val rotation: Int,
  /** 1 for the first frame, above 1 when the user magnified, rounded to 0.01. */
  val finalScale: Double,
  /** The move of the photo (of the frame when it is free) in whole pixels, right and down are positive. */
  val finalTranslateX: Int,
  val finalTranslateY: Int,
  /** True once any pan or pinch moved or scaled the crop. */
  val wasPositionChanged: Boolean
)

/**
 * The crop now without finishing it: [crop] and [rotation] are what `confirm()` would give at this moment, [finalScale] is 1 for the first frame (rounded to 0.01),
 * [finalTranslateX] and [finalTranslateY] are whole pixels, [wasPositionChanged] is true once any pan or pinch moved or scaled the crop.
 */
data class MediaCropSnapshot(
  val crop: RectF,
  val rotation: Int,
  val finalScale: Double,
  val finalTranslateX: Int,
  val finalTranslateY: Int,
  val wasPositionChanged: Boolean
)

/** The shape of the guide over the photo. The result is a rectangle in both. */
enum class MediaCropShape { Rect, Circle }

/**
 * The state of one crop. The app makes it ([rememberMediaCrop]), gives it to [MediaCropper] and calls [confirm] or [cancel] from its own buttons.
 * [aspect] fixes the proportions of the frame (the photo moves under a still frame), without it the frame is free (handles on the frame).
 * [onPositionChange] is called once per finished gesture with its [CropGestureReport]: the kind (`CropGesture.PINCH` when the fingers scaled at any time during it, otherwise `CropGesture.PAN`, a handle drag is a pan),
 * the length in milliseconds, the largest move, the changes of scale and position and the transform against the first frame. A touch that moved nothing gives no call.
 * [snapshot] gives the transform now without finishing, [cancel] gives it too.
 * [onFinish] is called once: with the result after [confirm], with null after [cancel]. A command after that, or before the photo is ready, is ignored.
 * Every call comes from the main thread.
 */
class MediaCrop(
  aspect: Size? = null,
  initial: RectF? = null,
  val allowsRotation: Boolean = true,
  private val limits: CropLimits = defaultCropLimits(),
  private val onPositionChange: ((CropGestureReport) -> Unit)? = null,
  private val onFinish: (MediaCropResult?) -> Unit
) {
  var isReady by mutableStateOf(false)
    private set
  var failure by mutableStateOf<MediaException?>(null)
    private set
  var isFinished by mutableStateOf(false)
    private set
  var rotation by mutableStateOf(0)
    private set
  var isGesturing by mutableStateOf(false)
    private set
  internal var rect by mutableStateOf(CropFractions(0.0, 0.0, 1.0, 1.0))
    private set

  internal val aspect: CropAspect? = aspect?.takeIf { it.width > 0 && it.height > 0 }?.let { CropAspect(it.width.toUInt(), it.height.toUInt()) }
  private val initial: CropFractions? = initial?.let { CropFractions(it.left.toDouble(), it.top.toDouble(), (it.right - it.left).toDouble(), (it.bottom - it.top).toDouble()) }
  private var baseline = CropFractions(0.0, 0.0, 1.0, 1.0)
  private var panning = false
  private var zooming = false
  private var positionChanged = false
  private var track: CropGestureTrack? = null
  private var gestureStart = 0L
  internal var clock: () -> Long = { System.nanoTime() / 1_000_000 }
  private var photo = PixelSize(0u, 0u)
  private var upright = PixelSize(0u, 0u)
  private var view = CropView(0.0, 0.0, 0.0)

  internal val setup: CropSetup get() = CropSetup(photo, this.aspect, limits)

  internal val layout: CropLayout? get() = if (isReady) cropLayout(rect, setup, view) else null

  /** The photo is on screen: [size] is its pixel size after the EXIF turn. */
  internal fun ready(size: PixelSize) {
    if (isFinished || size.width == 0u || size.height == 0u) return
    upright = size
    photo = size
    rotation = 0
    rect = cropInitial(setup, initial)
    baseline = rect
    failure = null
    isReady = true
  }

  internal fun fail(error: MediaException) {
    if (isFinished) return
    failure = error
    isReady = false
  }

  internal fun place(width: Int, height: Int, inset: Float) {
    view = CropView(width.toDouble(), height.toDouble(), inset.toDouble())
  }

  internal fun pan(dx: Float, dy: Float) {
    if (!isReady || isFinished) return
    if (dx != 0f || dy != 0f) panning = true
    rect = cropPanView(rect, setup, view, dx.toDouble(), dy.toDouble())
    follow()
  }

  internal fun zoom(factor: Float, at: PointF) {
    if (!isReady || isFinished) return
    if (factor.isFinite() && factor != 1f) zooming = true
    rect = cropZoomView(rect, setup, view, factor.toDouble(), at.x.toDouble(), at.y.toDouble())
    follow()
  }

  internal fun drag(handle: CropHandle, dx: Float, dy: Float) {
    if (!isReady || isFinished) return
    if (dx != 0f || dy != 0f) panning = true
    rect = cropDragView(rect, setup, view, handle, dx.toDouble(), dy.toDouble())
    follow()
  }

  internal fun handleAt(point: PointF, slop: Float): CropHandle? {
    if (aspect != null) return null
    val frame = layout?.frame ?: return null
    return cropHandleAt(frame, point.x.toDouble(), point.y.toDouble(), slop.toDouble())
  }

  internal fun gesture(active: Boolean) {
    if (isGesturing == active) return
    isGesturing = active
    if (active) {
      track = cropGestureBegin(rect)
      gestureStart = clock()
    } else {
      endGesture()
    }
  }

  private fun follow() {
    val open = track ?: return
    track = cropGestureMove(open, baseline, rect, setup, view)
  }

  private fun endGesture() {
    val open = track ?: return
    track = null
    val end = cropGestureEnd(open, baseline, rect, setup, view, panning, zooming, positionChanged, gestureStart.toULong(), clock().toULong())
    panning = false
    zooming = false
    positionChanged = end.wasPositionChanged
    end.reports.forEach { onPositionChange?.invoke(it) }
  }

  private fun transform(): MediaCropSnapshot? {
    val done = cropFinish(rect, setup) ?: return null
    val telemetry = cropTelemetry(baseline, rect, setup, view, positionChanged || panning || zooming)
    val crop = RectF(done.x.toFloat(), done.y.toFloat(), (done.x + done.width).toFloat(), (done.y + done.height).toFloat())
    return MediaCropSnapshot(crop, rotation, telemetry.finalScale, telemetry.finalTranslateX, telemetry.finalTranslateY, telemetry.wasPositionChanged)
  }

  /** The frame, the turn and the transform of the crop now, the crop stays open and `confirm()` still works. Null before the photo is ready. */
  fun snapshot(): MediaCropSnapshot? = if (isReady) transform() else null

  /** Turns the photo a quarter to the left (counter clockwise). Ignored when [allowsRotation] is false. */
  fun turnLeft() = turn(false)

  /** Turns the photo a quarter to the right (clockwise). Ignored when [allowsRotation] is false. */
  fun turnRight() = turn(true)

  /** The first frame and no turn again. */
  fun reset() {
    if (!isReady || isFinished) return
    photo = upright
    rotation = 0
    rect = cropInitial(setup, initial)
    baseline = rect
  }

  /** Finishes with the rectangle of the frame and the turn. Null when the photo is not ready or the crop is already finished. */
  fun confirm(): MediaCropResult? {
    if (!isReady || isFinished) return null
    val now = transform() ?: return null
    val result = MediaCropResult(
      now.crop,
      now.rotation,
      now.finalScale,
      now.finalTranslateX,
      now.finalTranslateY,
      now.wasPositionChanged
    )
    isFinished = true
    onFinish(result)
    return result
  }

  /** Finishes without a result and gives the transform at that moment (null when the photo was not ready or the crop was already finished). */
  fun cancel(): MediaCropSnapshot? {
    if (isFinished) return null
    val last = snapshot()
    isFinished = true
    onFinish(null)
    return last
  }

  private fun turn(clockwise: Boolean) {
    if (!allowsRotation || !isReady || isFinished) return
    rect = cropTurn(rect, setup, clockwise)
    baseline = cropTurn(baseline, setup, clockwise)
    photo = PixelSize(photo.height, photo.width)
    rotation = (rotation + if (clockwise) 90 else 270) % 360
  }

  /**
   * Decodes the photo for the screen and tells the crop its size. The decode takes its raster from the memory budget before it allocates ([MediaImageDecode]).
   * A source that is no local file, no image, or a refused budget is a [MediaException] in [failure]. Null then.
   */
  internal suspend fun load(source: MediaSource, maxSide: Int, admit: DecodeAdmit? = null): Bitmap? =
    try {
      val file = preparableFile(normalizeItem(source.toItem()).url)
      val bitmap = if (admit == null) MediaImageDecode.decode(file, maxSide) else decodeImage(ImageInput { file.inputStream() }, maxSide, admit)
      ready(PixelSize(bitmap.width.toUInt(), bitmap.height.toUInt()))
      bitmap
    } catch (cancelled: CancellationException) {
      throw cancelled
    } catch (error: MediaException) {
      fail(error)
      null
    }
}

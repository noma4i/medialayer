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
import com.noma4i.medialayer.cropHandleAt
import com.noma4i.medialayer.cropInitial
import com.noma4i.medialayer.cropLayout
import com.noma4i.medialayer.cropPanView
import com.noma4i.medialayer.cropTurn
import com.noma4i.medialayer.cropZoomView
import com.noma4i.medialayer.defaultCropLimits
import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.CancellationException

/**
 * What the user chose: [crop] is a rectangle in fractions 0 to 1 of the photo after [rotation] (origin top left),
 * [rotation] is the clockwise turn in degrees, 0, 90, 180 or 270. Give both to `MediaLayer.prepareImage(source, crop = crop, rotation = rotation)`.
 */
data class MediaCropResult(val crop: RectF, val rotation: Int)

/** The shape of the guide over the photo. The result is a rectangle in both. */
enum class MediaCropShape { Rect, Circle }

/**
 * The state of one crop. The app makes it ([rememberMediaCrop]), gives it to [MediaCropper] and calls [confirm] or [cancel] from its own buttons.
 * [aspect] fixes the proportions of the frame (the photo moves under a still frame), without it the frame is free (handles on the frame).
 * [onFinish] is called once: with the result after [confirm], with null after [cancel]. A command after that, or before the photo is ready, is ignored.
 * Every call comes from the main thread.
 */
class MediaCrop(
  aspect: Size? = null,
  initial: RectF? = null,
  val allowsRotation: Boolean = true,
  private val limits: CropLimits = defaultCropLimits(),
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
    if (isReady && !isFinished) rect = cropPanView(rect, setup, view, dx.toDouble(), dy.toDouble())
  }

  internal fun zoom(factor: Float, at: PointF) {
    if (isReady && !isFinished) rect = cropZoomView(rect, setup, view, factor.toDouble(), at.x.toDouble(), at.y.toDouble())
  }

  internal fun drag(handle: CropHandle, dx: Float, dy: Float) {
    if (isReady && !isFinished) rect = cropDragView(rect, setup, view, handle, dx.toDouble(), dy.toDouble())
  }

  internal fun handleAt(point: PointF, slop: Float): CropHandle? {
    if (aspect != null) return null
    val frame = layout?.frame ?: return null
    return cropHandleAt(frame, point.x.toDouble(), point.y.toDouble(), slop.toDouble())
  }

  internal fun gesture(active: Boolean) {
    if (isGesturing != active) isGesturing = active
  }

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
  }

  /** Finishes with the rectangle of the frame and the turn. Null when the photo is not ready or the crop is already finished. */
  fun confirm(): MediaCropResult? {
    if (!isReady || isFinished) return null
    val done = cropFinish(rect, setup) ?: return null
    val result = MediaCropResult(RectF(done.x.toFloat(), done.y.toFloat(), (done.x + done.width).toFloat(), (done.y + done.height).toFloat()), rotation)
    isFinished = true
    onFinish(result)
    return result
  }

  /** Finishes without a result. */
  fun cancel() {
    if (isFinished) return
    isFinished = true
    onFinish(null)
  }

  private fun turn(clockwise: Boolean) {
    if (!allowsRotation || !isReady || isFinished) return
    rect = cropTurn(rect, setup, clockwise)
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

package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.PointF
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.noma4i.medialayer.MediaSource
import kotlin.math.min
import kotlin.math.roundToInt

private const val HANDLE_SLOP_DP = 24
private const val SCREEN_ZOOM = 3

/**
 * The canvas of a crop: the photo, the dimmed area outside the frame, the frame and its guide, and the gestures.
 * A crop with proportions keeps the frame still and moves the photo (drag and pinch). A free crop moves the frame (drag a handle to resize, drag inside to move, pinch to resize).
 * The app owns the buttons and calls [MediaCrop.confirm] and [MediaCrop.cancel]. The source is a local file. The composable does no work while the finger is up.
 */
@Composable
fun MediaCropper(source: MediaSource, crop: MediaCrop, modifier: Modifier = Modifier, shape: MediaCropShape = MediaCropShape.Rect, inset: Dp = 16.dp) {
  val density = LocalDensity.current
  val insetPx = with(density) { inset.toPx() }
  val slopPx = with(density) { HANDLE_SLOP_DP.dp.toPx() }
  var bitmap by remember(source.url) { mutableStateOf<Bitmap?>(null) }
  var size by remember { mutableStateOf(IntSize.Zero) }
  LaunchedEffect(source.url, size) {
    if (size == IntSize.Zero) return@LaunchedEffect
    crop.place(size.width, size.height, insetPx)
    if (bitmap == null) bitmap = crop.load(source, min(4096, maxOf(size.width, size.height) * SCREEN_ZOOM).coerceAtLeast(256))
  }
  val shown = bitmap
  DisposableEffect(shown) { onDispose { shown?.recycle() } }
  val image = remember(shown) { shown?.asImageBitmap() }
  Box(
    modifier.fillMaxSize().background(Color.Black).clipToBounds().onSizeChanged { size = it }.semantics { testTag = "media-cropper" }
      .pointerInput(crop) {
        awaitEachGesture {
          val down = awaitFirstDown(requireUnconsumed = false)
          val handle = crop.handleAt(PointF(down.position.x, down.position.y), slopPx)
          crop.gesture(true)
          do {
            val event = awaitPointerEvent()
            val zoom = event.calculateZoom()
            val pan = event.calculatePan()
            val centroid = event.calculateCentroid()
            if (zoom != 1f && centroid.isSpecified) crop.zoom(zoom, PointF(centroid.x, centroid.y))
            if (pan != Offset.Zero) if (handle != null) crop.drag(handle, pan.x, pan.y) else crop.pan(pan.x, pan.y)
            event.changes.forEach { if (it.positionChanged()) it.consume() }
          } while (event.changes.any { it.pressed })
          crop.gesture(false)
        }
      }
  ) {
    val layout = crop.layout
    if (image != null && layout != null) {
      Canvas(Modifier.fillMaxSize()) {
        val photo = Rect(layout.photo.x.toFloat(), layout.photo.y.toFloat(), (layout.photo.x + layout.photo.width).toFloat(), (layout.photo.y + layout.photo.height).toFloat())
        val sideways = crop.rotation % 180 == 90
        val drawn = IntSize((if (sideways) photo.height else photo.width).roundToInt(), (if (sideways) photo.width else photo.height).roundToInt())
        rotate(crop.rotation.toFloat(), photo.center) {
          drawImage(image, dstOffset = IntOffset((photo.center.x - drawn.width / 2f).roundToInt(), (photo.center.y - drawn.height / 2f).roundToInt()), dstSize = drawn)
        }
        val frame = Rect(layout.frame.x.toFloat(), layout.frame.y.toFloat(), (layout.frame.x + layout.frame.width).toFloat(), (layout.frame.y + layout.frame.height).toFloat())
        val dim = Path().apply {
          fillType = PathFillType.EvenOdd
          addRect(Rect(Offset.Zero, this@Canvas.size))
          if (shape == MediaCropShape.Circle) addOval(frame) else addRect(frame)
        }
        drawPath(dim, Color.Black.copy(alpha = 0.55f))
        if (shape == MediaCropShape.Circle) drawOval(Color.White, frame.topLeft, frame.size, style = Stroke(1.5.dp.toPx()))
        else drawRect(Color.White, frame.topLeft, frame.size, style = Stroke(1.5.dp.toPx()))
        if (crop.isGesturing) {
          for (step in listOf(1f / 3, 2f / 3)) {
            drawLine(Color.White.copy(alpha = 0.6f), Offset(frame.left + frame.width * step, frame.top), Offset(frame.left + frame.width * step, frame.bottom), 0.5.dp.toPx())
            drawLine(Color.White.copy(alpha = 0.6f), Offset(frame.left, frame.top + frame.height * step), Offset(frame.right, frame.top + frame.height * step), 0.5.dp.toPx())
          }
        }
      }
    }
  }
}

/** A crop for the composition: made once, with the choices it was made with. */
@Composable
fun rememberMediaCrop(
  aspect: android.util.Size? = null,
  initial: android.graphics.RectF? = null,
  allowsRotation: Boolean = true,
  onPositionChange: ((com.noma4i.medialayer.CropGesture) -> Unit)? = null,
  onFinish: (MediaCropResult?) -> Unit
): MediaCrop {
  val latest by androidx.compose.runtime.rememberUpdatedState(onFinish)
  val latestPosition by androidx.compose.runtime.rememberUpdatedState(onPositionChange)
  return remember(aspect, initial, allowsRotation) { MediaCrop(aspect, initial, allowsRotation, onPositionChange = { latestPosition?.invoke(it) }) { latest(it) } }
}

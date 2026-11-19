package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Color
import androidx.camera.view.PreviewView
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.Observer
import androidx.lifecycle.findViewTreeLifecycleOwner
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CaptureProfile

/**
 * The preview of a camera. The camera runs while this composable is in the composition and its lifecycle is started. The picture fills the frame (`FILL_CENTER`).
 * A pinch zooms the camera when [zoomsByPinch] is true.
 */
@Composable
fun MediaCameraPreview(camera: MediaCamera, modifier: Modifier = Modifier, zoomsByPinch: Boolean = true) {
  val owner = LocalView.current.findViewTreeLifecycleOwner()
  val context = LocalContext.current
  val view = remember(context) { cameraPreviewView(context) }
  DisposableEffect(camera, owner, view) {
    if (owner == null) return@DisposableEffect onDispose { }
    val observer = LifecycleEventObserver { _, event ->
      when (event) {
        Lifecycle.Event.ON_START -> camera.attachPreview(view, view, owner)
        Lifecycle.Event.ON_STOP -> camera.detachPreview(view)
        else -> Unit
      }
    }
    owner.lifecycle.addObserver(observer)
    onDispose {
      owner.lifecycle.removeObserver(observer)
      camera.detachPreview(view)
    }
  }
  // The transparent view draws nothing until its first frame, so what lies under it stays visible; the stream state is only traced.
  DisposableEffect(view, owner) {
    if (owner == null) return@DisposableEffect onDispose { }
    val observer = Observer<PreviewView.StreamState> { applyStreamState(view, it) }
    view.previewStreamState.observe(owner, observer)
    onDispose { view.previewStreamState.removeObserver(observer) }
  }
  val pinch = if (zoomsByPinch) Modifier.pointerInput(camera) { detectTransformGestures { _, _, zoom, _ -> if (zoom != 1f) camera.zoomBy(zoom.toDouble()) } } else Modifier
  AndroidView(factory = { view }, modifier = modifier.then(pinch))
}

internal fun cameraPreviewView(context: Context): PreviewView = PreviewView(context).apply {
  implementationMode = PreviewView.ImplementationMode.COMPATIBLE
  scaleType = PreviewView.ScaleType.FILL_CENTER
  setBackgroundColor(Color.TRANSPARENT)
}

/** Traces when the preview starts and stops streaming; the view itself stays shown, its transparent background and TextureView draw nothing before the first frame. */
internal fun applyStreamState(view: PreviewView, state: PreviewView.StreamState) {
  val streaming = state == PreviewView.StreamState.STREAMING
  MediaTrace.emit("camera") { if (streaming) "preview shown" else "preview hidden" }
}

/** A camera for the composition: made once for [profile] and [lens], closed when it leaves the composition. */
@Composable
fun rememberMediaCamera(profile: CaptureProfile, lens: CameraLens = CameraLens.BACK): MediaCamera {
  val context = LocalContext.current.applicationContext
  val camera = remember(profile, lens) { MediaCamera(context, profile, lens) }
  DisposableEffect(camera) { onDispose { camera.close() } }
  return camera
}

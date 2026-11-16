package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.hardware.camera2.CameraCharacteristics
import androidx.camera.camera2.interop.Camera2CameraInfo
import androidx.camera.camera2.interop.ExperimentalCamera2Interop
import androidx.camera.core.Camera
import androidx.camera.core.CameraInfo
import androidx.camera.core.CameraSelector
import androidx.camera.core.CameraState
import androidx.camera.core.DynamicRange
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.MirrorMode
import androidx.camera.core.Preview
import androidx.camera.core.UseCase
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FallbackStrategy
import androidx.camera.video.AudioStats
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.Observer
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraRefusal
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureMode
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.FaultKind
import com.noma4i.medialayer.FpsRange
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.RecordEnd
import com.noma4i.medialayer.VideoStabilization
import com.noma4i.medialayer.formatCandidates
import com.noma4i.medialayer.pickFormat
import com.noma4i.medialayer.stabilizationMode
import java.io.File
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/**
 * The hardware camera: CameraX use cases bound to the lifecycle of the preview. The binding runs on Main (CameraX requires it) and never waits for a future.
 * The format of the profile is picked by the one rule of the core, from the sizes this camera offers.
 */
@androidx.annotation.OptIn(markerClass = [ExperimentalCamera2Interop::class])
internal class CameraXBackend(private val context: Context) : CaptureBackend {
  private class Plan(val quality: Quality, val photoSize: PixelSize?, val frameRate: FpsRange)

  private val main = ContextCompat.getMainExecutor(context)
  @Volatile private var sink: (CaptureEvent) -> Unit = {}
  @Volatile private var audioIssue: (CaptureAudioIssue) -> Unit = {}

  override fun setAudioIssueSink(sink: (CaptureAudioIssue) -> Unit) { audioIssue = sink }
  private var view: PreviewView? = null
  private var owner: LifecycleOwner? = null
  private var cameras: ProcessCameraProvider? = null
  private var camera: Camera? = null
  private var preview: Preview? = null
  private var photo: ImageCapture? = null
  private var movie: VideoCapture<Recorder>? = null
  private var analysis: ImageAnalysis? = null
  private var recording: Recording? = null
  private var analyzer: ImageAnalysis.Analyzer? = null
  private var lastBind: Pair<CameraLens, CaptureProfile>? = null
  private var watchingStream = false
  @Volatile private var recordedNanos = 0L
  @Volatile override var photoSize: PixelSize? = null
    private set

  private val cameraObserver = Observer<CameraState> { state ->
    when {
      state.error != null -> sink(CaptureEvent.Fault(if (isRecoverable(state.error!!.code)) FaultKind.RESET else FaultKind.OTHER))
      state.type == CameraState.Type.OPEN -> announceOpened()
      else -> sink(CaptureEvent.CameraClosed)
    }
  }
  private val streamObserver = Observer<PreviewView.StreamState> { stream -> sink(CaptureEvent.Streaming(stream == PreviewView.StreamState.STREAMING)) }

  override fun setSink(sink: (CaptureEvent) -> Unit) {
    this.sink = sink
  }

  override fun setHost(view: PreviewView?, owner: LifecycleOwner?) {
    this.view = view
    this.owner = owner
  }

  override fun sensorToView(): Matrix? = view?.sensorToViewTransform

  override fun bind(lens: CameraLens, profile: CaptureProfile) {
    lastBind = lens to profile
    main.execute { bindNow(lens, profile) }
  }

  private fun granted(permission: String) = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

  private fun bindNow(lens: CameraLens, profile: CaptureProfile) {
    val host = view
    val lifecycle = owner
    if (host == null || lifecycle == null) return sink(CaptureEvent.Refuse(CameraRefusal.CONFIGURE_FAILED))
    if (!granted(Manifest.permission.CAMERA) || (profile.audio && !granted(Manifest.permission.RECORD_AUDIO))) {
      return sink(CaptureEvent.Refuse(CameraRefusal.PERMISSION_DENIED))
    }
    val future = ProcessCameraProvider.getInstance(context)
    future.addListener({
      try {
        bindOn(future.get(), lens, profile, host, lifecycle)
      } catch (_: Exception) {
        sink(CaptureEvent.Refuse(CameraRefusal.CONFIGURE_FAILED))
      }
    }, main)
  }

  private fun bindOn(provider: ProcessCameraProvider, lens: CameraLens, profile: CaptureProfile, host: PreviewView, lifecycle: LifecycleOwner) {
    cameras = provider
    release(provider)
    val selector = if (lens == CameraLens.FRONT) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
    if (!provider.hasCamera(selector)) return sink(CaptureEvent.Refuse(CameraRefusal.NO_CAMERA))
    val plan = plan(provider.getCameraInfo(selector), profile)
    photoSize = plan?.photoSize
    val made = Preview.Builder().setResolutionSelector(previewSelector(plan, profile)).build().apply { surfaceProvider = host.surfaceProvider }
    val cases = mutableListOf<UseCase>(made)
    preview = made
    if (profile.mode == CaptureMode.PHOTO) {
      val capture = ImageCapture.Builder().setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY).setJpegQuality(profile.photoQuality.toInt())
        .setResolutionSelector(photoSelector(plan)).build()
      photo = capture
      cases += capture
    } else {
      val quality = plan?.quality ?: Quality.HD
      val builder = VideoCapture.Builder(Recorder.Builder().setQualitySelector(QualitySelector.from(quality, FallbackStrategy.lowerQualityOrHigherThan(quality))).build())
        .setMirrorMode(if (profile.mirrorFrontVideo) MirrorMode.MIRROR_MODE_ON_FRONT_ONLY else MirrorMode.MIRROR_MODE_OFF)
      plan?.let { builder.setTargetFrameRate(android.util.Range(it.frameRate.low.toInt(), it.frameRate.high.toInt())) }
      val offered = if (Recorder.getVideoCapabilities(provider.getCameraInfo(selector)).isStabilizationSupported) listOf(VideoStabilization.STANDARD, VideoStabilization.CINEMATIC) else emptyList()
      stabilizationMode(profile.stabilization, offered)?.let { builder.setVideoStabilizationEnabled(it != VideoStabilization.OFF) }
      val capture = builder.build()
      movie = capture
      cases += capture
    }
    analyzer?.let { tap ->
      val use = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build().apply { setAnalyzer(main, tap) }
      analysis = use
      cases += use
    }
    val active = provider.bindToLifecycle(lifecycle, selector, *cases.toTypedArray())
    camera = active
    if (profile.mode == CaptureMode.PHOTO && profile.reducesRedEye) enableRedEyeReduction(active)
    active.cameraInfo.cameraState.observe(lifecycle, cameraObserver)
    if (!watchingStream) {
      watchingStream = true
      host.previewStreamState.observe(lifecycle, streamObserver)
    }
  }

  private fun enableRedEyeReduction(bound: Camera) {
    val modes = Camera2CameraInfo.from(bound.cameraInfo).getCameraCharacteristic(CameraCharacteristics.CONTROL_AE_AVAILABLE_MODES) ?: return
    if (AE_MODE_ON_AUTO_FLASH_REDEYE !in modes) return
    androidx.camera.camera2.interop.Camera2CameraControl.from(bound.cameraControl).addCaptureRequestOptions(
      androidx.camera.camera2.interop.CaptureRequestOptions.Builder()
        .setCaptureRequestOption(android.hardware.camera2.CaptureRequest.CONTROL_AE_MODE, AE_MODE_ON_AUTO_FLASH_REDEYE).build()
    )
  }

  private fun announceOpened() {
    val bound = camera ?: return
    val zoom = bound.cameraInfo.zoomState.value
    sink(CaptureEvent.Opened(bound.cameraInfo.hasFlashUnit(), zoom?.minZoomRatio ?: 1f, zoom?.maxZoomRatio ?: 1f))
  }

  private fun release(provider: ProcessCameraProvider) {
    camera?.let { it.cameraInfo.cameraState.removeObserver(cameraObserver) }
    val cases = listOfNotNull(preview, photo, movie, analysis)
    if (cases.isNotEmpty()) provider.unbind(*cases.toTypedArray())
    preview = null
    photo = null
    movie = null
    analysis = null
    camera = null
  }

  override fun unbind() {
    lastBind = null
    main.execute {
      recording?.close()
      recording = null
      cameras?.let(::release)
      if (watchingStream) {
        view?.previewStreamState?.removeObserver(streamObserver)
        watchingStream = false
      }
    }
  }

  override suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile) {
    val capture = photo ?: throw IllegalStateException("no photo use case is bound")
    withContext(Dispatchers.Main.immediate) {
      capture.flashMode = if (flash && camera?.cameraInfo?.hasFlashUnit() == true) ImageCapture.FLASH_MODE_ON else ImageCapture.FLASH_MODE_OFF
      suspendCancellableCoroutine<Unit> { continuation ->
        capture.takePicture(ImageCapture.OutputFileOptions.Builder(file).build(), main, object : ImageCapture.OnImageSavedCallback {
          override fun onImageSaved(outputFileResults: ImageCapture.OutputFileResults) = continuation.resume(Unit)

          override fun onError(exception: ImageCaptureException) = continuation.resumeWithException(exception)
        })
      }
    }
  }

  override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens) {
    main.execute {
      val output = movie?.output ?: return@execute sink(CaptureEvent.RecordFinished(RecordEnd.FAILED))
      try {
        recordedNanos = 0
        var pending = output.prepareRecording(context, FileOutputOptions.Builder(file).setDurationLimitMillis(profile.maxDurationMs.toLong()).build())
        if (profile.audio) pending = pending.withAudioEnabled()
        MediaTrace.emit("camera") { "take audio=${profile.audio}" }
        recording = pending.start(main) { event ->
          if (profile.audio) {
            when (event.recordingStats.audioStats.audioState) {
              AudioStats.AUDIO_STATE_SOURCE_SILENCED -> audioIssue(CaptureAudioIssue.SILENCED)
              AudioStats.AUDIO_STATE_ACTIVE -> audioIssue(CaptureAudioIssue.ACTIVE)
              AudioStats.AUDIO_STATE_SOURCE_ERROR -> audioIssue(CaptureAudioIssue.SOURCE_ERROR)
              AudioStats.AUDIO_STATE_ENCODER_ERROR -> audioIssue(CaptureAudioIssue.ENCODER_ERROR)
            }
          }
          when (event) {
            is VideoRecordEvent.Start -> sink(CaptureEvent.RecordStarted)
            is VideoRecordEvent.Status -> recordedNanos = event.recordingStats.recordedDurationNanos
            is VideoRecordEvent.Finalize -> {
              recording = null
              sink(CaptureEvent.RecordFinished(endOf(event.error)))
            }
            else -> Unit
          }
        }
      } catch (_: SecurityException) {
        audioIssue(CaptureAudioIssue.PERMISSION_DENIED)
        sink(CaptureEvent.RecordFinished(RecordEnd.FAILED))
      } catch (_: Exception) {
        sink(CaptureEvent.RecordFinished(RecordEnd.FAILED))
      }
    }
  }

  override fun endRecording() {
    main.execute { recording?.stop() }
  }

  override fun recordedMs(): Long = recordedNanos / 1_000_000

  override fun setTorch(on: Boolean) {
    main.execute { camera?.cameraControl?.enableTorch(on) }
  }

  override fun setZoom(ratio: Float) {
    main.execute { camera?.cameraControl?.setZoomRatio(ratio) }
  }

  override fun setFrameTap(analyzer: ImageAnalysis.Analyzer?) {
    main.execute {
      if (this.analyzer === analyzer) return@execute
      this.analyzer = analyzer
      lastBind?.let { (lens, profile) -> if (camera != null) bindNow(lens, profile) }
    }
  }

  private fun plan(info: CameraInfo, profile: CaptureProfile): Plan? {
    val camera2 = Camera2CameraInfo.from(info)
    val map = camera2.getCameraCharacteristic(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return null
    val jpeg = map.getOutputSizes(ImageFormat.JPEG).orEmpty().map { PixelSize(it.width.toUInt(), it.height.toUInt()) }
    val rates = camera2.getCameraCharacteristic(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty().map { FpsRange(it.lower.toUInt(), it.upper.toUInt()) }
    val supported = Recorder.getVideoCapabilities(info).getSupportedQualities(DynamicRange.SDR)
    val formats = QUALITIES.filter { it in supported }.mapNotNull { quality ->
      QualitySelector.getResolution(info, quality)?.let { quality to PixelSize(it.width.toUInt(), it.height.toUInt()) }
    }
    val choice = pickFormat(formatCandidates(formats.map { it.second }, jpeg, rates), profile) ?: return null
    return Plan(formats[choice.index.toInt()].first, choice.photoSize, choice.frameRate)
  }

  private fun aspect(size: PixelSize): AspectRatioStrategy {
    val wide = size.width.toDouble() / size.height.toDouble()
    val ratio = if (wide >= 1) wide else 1 / wide
    return if (kotlin.math.abs(ratio - 16.0 / 9) / (16.0 / 9) < ASPECT_TOLERANCE) AspectRatioStrategy.RATIO_16_9_FALLBACK_AUTO_STRATEGY else AspectRatioStrategy.RATIO_4_3_FALLBACK_AUTO_STRATEGY
  }

  private fun previewSelector(plan: Plan?, profile: CaptureProfile): ResolutionSelector {
    val builder = ResolutionSelector.Builder()
    (plan?.photoSize ?: profile.videoSize)?.let { builder.setAspectRatioStrategy(aspect(it)) }
    return builder.build()
  }

  private fun photoSelector(plan: Plan?): ResolutionSelector {
    val builder = ResolutionSelector.Builder()
    val size = plan?.photoSize ?: return builder.build()
    builder.setAspectRatioStrategy(aspect(size))
    builder.setResolutionFilter { supported, _ ->
      val exact = supported.filter { (it.width.toUInt() == size.width && it.height.toUInt() == size.height) || (it.width.toUInt() == size.height && it.height.toUInt() == size.width) }
      exact + (supported - exact.toSet())
    }
    return builder.build()
  }

  private fun endOf(error: Int) = when (error) {
    VideoRecordEvent.Finalize.ERROR_NONE -> RecordEnd.COMPLETED
    VideoRecordEvent.Finalize.ERROR_DURATION_LIMIT_REACHED -> RecordEnd.LIMIT_REACHED
    else -> RecordEnd.FAILED
  }

  private fun isRecoverable(code: Int) = code == CameraState.ERROR_CAMERA_IN_USE || code == CameraState.ERROR_MAX_CAMERAS_IN_USE || code == CameraState.ERROR_OTHER_RECOVERABLE_ERROR

  private companion object {
    const val AE_MODE_ON_AUTO_FLASH_REDEYE = 4
    const val ASPECT_TOLERANCE = 0.02
    val QUALITIES = listOf(Quality.SD, Quality.HD, Quality.FHD, Quality.UHD)
  }
}

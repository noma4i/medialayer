package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.hardware.camera2.CameraCharacteristics
import android.media.AudioFormat
import android.media.CamcorderProfile
import android.media.EncoderProfiles
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.media.MediaRecorder
import android.util.Size
import android.view.Surface
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
import androidx.camera.core.Preview
import androidx.camera.core.UseCase
import androidx.camera.core.UseCaseGroup
import androidx.camera.core.impl.CameraInfoInternal
import androidx.camera.core.impl.EncoderProfilesProxy
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
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

internal class Encoders(
  val videoMime: String,
  val width: Int,
  val height: Int,
  val videoBitrate: Int,
  val frameRate: Int,
  val audioMime: String?,
  val audioProfile: Int,
  val audioBitrate: Int,
  val sampleRate: Int,
  val channels: Int,
  val timebase: VideoTimebase
) {
  fun audioFormat(): MediaFormat? = audioMime?.let { mime ->
    MediaFormat.createAudioFormat(mime, sampleRate, channels).apply {
      if (mime == MediaFormat.MIMETYPE_AUDIO_AAC && audioProfile != NO_CODEC_PROFILE) setInteger(MediaFormat.KEY_AAC_PROFILE, audioProfile)
      setInteger(MediaFormat.KEY_BIT_RATE, audioBitrate)
    }
  }

  fun withAudioProfile(profile: Int) = Encoders(videoMime, width, height, videoBitrate, frameRate, audioMime, profile, audioBitrate, sampleRate, channels, timebase)

  fun settings(audio: Boolean, upright: Size, audioGeneration: Long, heldLimitBytes: Long, maxDurationMs: Long): CameraTakeSettings {
    val video = MediaFormat.createVideoFormat(videoMime, upright.width, upright.height).apply {
      setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
      setInteger(MediaFormat.KEY_BIT_RATE, videoBitrate)
      setInteger(MediaFormat.KEY_FRAME_RATE, frameRate)
      setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, INTRA_FRAME_INTERVAL_SECONDS)
    }
    return CameraTakeSettings(video, audioFormat()?.takeIf { audio }, timebase, audioGeneration, heldLimitBytes, maxDurationMs * 1000)
  }
}

private const val SDR_BIT_DEPTH = 8
private const val INTRA_FRAME_INTERVAL_SECONDS = 1
private const val NO_CODEC_PROFILE = -1
private const val AUDIO_SAMPLE_RATE_DEFAULT = 44_100
private const val AUDIO_CHANNEL_COUNT_DEFAULT = 1
private const val AUDIO_BITRATE_BASE = 156_000
private const val AUDIO_CHANNEL_COUNT_BASE = 2
private const val AUDIO_SAMPLE_RATE_BASE = 48_000

/** The encoders of a profile set: the first 8-bit SDR video profile and the AAC profile, or the default AAC of the core rule when the set holds none. Null when the set holds no SDR video. */
internal fun encodersFrom(all: EncoderProfilesProxy?, audio: Boolean, timebase: VideoTimebase): Encoders? {
  val video = all?.videoProfiles?.firstOrNull { it.bitDepth == SDR_BIT_DEPTH && it.hdrFormat == EncoderProfiles.VideoProfile.HDR_NONE } ?: return null
  val sound = all.audioProfiles.firstOrNull { it.mediaType == MediaFormat.MIMETYPE_AUDIO_AAC }
  return if (!audio) {
    Encoders(video.mediaType, video.width, video.height, video.bitrate, video.frameRate, null, NO_CODEC_PROFILE, 0, 0, 0, timebase)
  } else if (sound != null) {
    Encoders(video.mediaType, video.width, video.height, video.bitrate, video.frameRate, sound.mediaType, sound.profile, sound.bitrate, sound.sampleRate, sound.channels, timebase)
  } else {
    val bitrate = (AUDIO_BITRATE_BASE * (AUDIO_CHANNEL_COUNT_DEFAULT.toDouble() / AUDIO_CHANNEL_COUNT_BASE) * (AUDIO_SAMPLE_RATE_DEFAULT.toDouble() / AUDIO_SAMPLE_RATE_BASE)).toInt()
    Encoders(video.mediaType, video.width, video.height, video.bitrate, video.frameRate, MediaFormat.MIMETYPE_AUDIO_AAC, MediaCodecInfo.CodecProfileLevel.AACObjectLC, bitrate, AUDIO_SAMPLE_RATE_DEFAULT, AUDIO_CHANNEL_COUNT_DEFAULT, timebase)
  }
}

/**
 * The AAC profile of the audio encoder: the one of the camera profile when an AAC encoder of the device declares it, else AAC-LC when one declares that, else null.
 * Follows the CameraX recorder, which always encodes AAC as AAC-LC (AudioConfigUtil.audioMimeToAudioProfile, AAC_DEFAULT_PROFILE = AACObjectLC, in
 * androidx/camera/camera-video/src/main/java/androidx/camera/video/internal/config/AudioConfigUtil.kt) and takes only the rate, channels and bitrate from the camera profile.
 */
internal fun resolveAacProfile(declared: Int, supported: Set<Int>): Int? = when {
  declared in supported -> declared
  MediaCodecInfo.CodecProfileLevel.AACObjectLC in supported -> MediaCodecInfo.CodecProfileLevel.AACObjectLC
  else -> null
}

/** The step of the plan the camera stopped at: each one is its own text in the failure, so a device report tells which one it was. */
internal enum class PlanRefusal(val step: String) {
  NO_STREAM_MAP("the camera has no SCALER_STREAM_CONFIGURATION_MAP"),
  NO_FORMAT("no format of the profile fits the sizes of the camera"),
  NO_SDR_PROFILE("the camera has no 8-bit SDR video profile for the quality"),
  NO_AUDIO_ENCODER("the device has no AAC encoder for the audio format")
}

/** What the camera offered when the plan was refused; null where the step was not reached. */
internal class PlanFacts(val cameraId: String, val quality: String?, val supported: List<String>, val videoProfiles: Int?, val audioProfiles: Int?, val audio: Encoders?)

internal class PlanRefused(val reason: PlanRefusal, facts: PlanFacts) : IllegalStateException(describe(reason, facts))

private fun describe(reason: PlanRefusal, facts: PlanFacts): String {
  val sound = facts.audio?.let { "audio ${it.audioMime} ${it.sampleRate}Hz ${it.channels}ch profile ${it.audioProfile} ${it.audioBitrate}bps" } ?: "audio none"
  return "${reason.step}: camera ${facts.cameraId}, quality ${facts.quality ?: "none"}, supported ${facts.supported}, " +
    "profiles video ${facts.videoProfiles ?: "none"} audio ${facts.audioProfiles ?: "none"}, $sound"
}

internal fun uprightVertex(rotationDegrees: Int, mirror: Boolean): FloatArray = FloatArray(16).also {
  android.opengl.Matrix.setIdentityM(it, 0)
  if (mirror) android.opengl.Matrix.scaleM(it, 0, -1f, 1f, 1f)
  android.opengl.Matrix.rotateM(it, 0, -rotationDegrees.toFloat(), 0f, 0f, 1f)
}

/**
 * The hardware camera: CameraX use cases bound to the lifecycle of the preview. The binding runs on Main (CameraX requires it) and never waits for a future.
 * The format of the profile is picked by the one rule of the core, from the sizes this camera offers.
 */
@androidx.annotation.OptIn(markerClass = [ExperimentalCamera2Interop::class])
internal class CameraXBackend(private val context: Context, private val audioLink: () -> AudioLink? = { Media.link }) : CaptureBackend {
  private class Plan(val quality: Quality, val photoSize: PixelSize?, val frameRate: FpsRange, val encoders: Encoders?)

  private val main = ContextCompat.getMainExecutor(context)
  private val calculations = java.util.concurrent.Executors.newSingleThreadExecutor()
  private val session = java.util.concurrent.atomic.AtomicLong()
  private val take = java.util.concurrent.atomic.AtomicLong()
  @Volatile private var sink: (CaptureEvent) -> Unit = {}
  @Volatile private var audioIssue: (CaptureAudioIssue) -> Unit = {}

  override fun setAudioIssueSink(sink: (CaptureAudioIssue) -> Unit) { audioIssue = sink }
  private var view: PreviewView? = null
  private var owner: LifecycleOwner? = null
  private var cameras: ProcessCameraProvider? = null
  private var camera: Camera? = null
  private var preview: Preview? = null
  private var photo: ImageCapture? = null
  @Volatile private var output: CameraVideoOutput? = null
  private var encoders: Encoders? = null
  @Volatile private var memory: CaptureMemory? = null
  private var analysis: ImageAnalysis? = null
  private val recording = CameraRecording(main, { sink(it) }) { generation, file, settings, listener -> CameraTake(generation, file, settings, leases(), listener) }
  private val audio = CameraAudioHold({ audioLink() }, recording, { audioIssue(it) }) { listener -> RecordingAudio(listener, ::lease, main).asMicrophone() }
  private var analyzer: ImageAnalysis.Analyzer? = null
  private var lastBind: Pair<CameraLens, CaptureProfile>? = null
  private var watchingStream = false
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

  override fun useMemory(memory: CaptureMemory) { this.memory = memory }

  private fun leases(): (Long) -> AutoCloseable = checkNotNull(memory) { "no capture memory is admitted" }::lease

  // The microphone asks the memory of the session that is current when it starts, never the one that was current when it was made.
  private fun lease(bytes: Long): AutoCloseable = checkNotNull(memory) { "no capture memory is admitted" }.lease(bytes)

  override fun sensorToView(): Matrix? = view?.sensorToViewTransform

  override fun bind(lens: CameraLens, profile: CaptureProfile) {
    lastBind = lens to profile
    val generation = session.incrementAndGet()
    main.execute { bindNow(lens, profile, generation) }
  }

  private fun granted(permission: String) = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

  private fun bindNow(lens: CameraLens, profile: CaptureProfile, generation: Long) {
    if (generation != session.get()) return
    val host = view
    val lifecycle = owner
    if (host == null || lifecycle == null) return sink(CaptureEvent.Refuse(CameraRefusal.CONFIGURE_FAILED))
    if (!granted(Manifest.permission.CAMERA) || (profile.audio && !granted(Manifest.permission.RECORD_AUDIO))) {
      return sink(CaptureEvent.Refuse(CameraRefusal.PERMISSION_DENIED))
    }
    val future = ProcessCameraProvider.getInstance(context)
    future.addListener({
      try {
        if (generation != session.get()) return@addListener
        val provider = future.get()
        val selector = if (lens == CameraLens.FRONT) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
        if (!provider.hasCamera(selector)) return@addListener sink(CaptureEvent.Refuse(CameraRefusal.NO_CAMERA))
        val info = provider.getCameraInfo(selector)
        calculations.execute {
          try {
            val plan = plan(info, profile)
            val offered = if (Preview.getPreviewCapabilities(info).isStabilizationSupported) listOf(VideoStabilization.STANDARD, VideoStabilization.CINEMATIC) else emptyList()
            val stabilization = stabilizationMode(profile.stabilization, offered)
            main.execute {
              if (generation == session.get()) try { bindOn(provider, lens, profile, host, lifecycle, plan, stabilization, generation) } catch (error: Exception) { configureFailed(error, generation) }
            }
          } catch (error: Exception) { configureFailed(error, generation) }
        }
      } catch (error: Exception) {
        configureFailed(error.cause ?: error, generation)
      }
    }, main)
  }

  private fun configureFailed(error: Throwable, generation: Long) {
    main.execute {
      if (generation != session.get()) return@execute
      audio.keep(null)
      Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, error)
      sink(CaptureEvent.Refuse(CameraRefusal.CONFIGURE_FAILED))
    }
  }

  private fun bindOn(provider: ProcessCameraProvider, lens: CameraLens, profile: CaptureProfile, host: PreviewView, lifecycle: LifecycleOwner, plan: Result<Plan>, stabilization: VideoStabilization?, generation: Long) {
    cameras = provider
    release(provider)
    val selector = if (lens == CameraLens.FRONT) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
    if (!provider.hasCamera(selector)) return sink(CaptureEvent.Refuse(CameraRefusal.NO_CAMERA))
    val ready = if (profile.mode == CaptureMode.VIDEO) plan.getOrElse { return configureFailed(it, generation) } else plan.getOrNull()
    photoSize = ready?.photoSize
    val recorded = ready?.encoders.takeIf { profile.mode == CaptureMode.VIDEO }
    val builder = Preview.Builder().setResolutionSelector(previewSelector(ready, profile, recorded))
    if (recorded != null) {
      ready?.let { builder.setTargetFrameRate(android.util.Range(it.frameRate.low.toInt(), it.frameRate.high.toInt())) }
      stabilization?.let { builder.setPreviewStabilizationEnabled(it != VideoStabilization.OFF) }
    }
    val made = builder.build().apply { surfaceProvider = host.surfaceProvider }
    val cases = mutableListOf<UseCase>(made)
    preview = made
    if (profile.mode == CaptureMode.PHOTO) {
      val capture = ImageCapture.Builder().setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY).setJpegQuality(profile.photoQuality.toInt())
        .setResolutionSelector(photoSelector(ready)).build()
      photo = capture
      cases += capture
    }
    audio.keep(if (recorded != null && profile.audio) pcmSpecOf(recorded) else null)
    val effect = recorded?.let {
      encoders = it
      CameraVideoOutput(it.timebase, leases(), { generation, failure -> recording.encoderFailed(generation, failure) }, ::outputFailed).also { made -> output = made }.effect
    }
    analyzer?.let { tap ->
      val use = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build().apply { setAnalyzer(main, tap) }
      analysis = use
      cases += use
    }
    val active = if (effect == null) provider.bindToLifecycle(lifecycle, selector, *cases.toTypedArray()) else {
      provider.bindToLifecycle(lifecycle, selector, UseCaseGroup.Builder().apply { cases.forEach { addUseCase(it) } }.addEffect(effect).build())
    }
    camera = active
    if (profile.mode == CaptureMode.PHOTO && profile.reducesRedEye) enableRedEyeReduction(active)
    active.cameraInfo.cameraState.observe(lifecycle, cameraObserver)
    if (!watchingStream) {
      watchingStream = true
      host.previewStreamState.observe(lifecycle, streamObserver)
    }
  }

  private fun pcmSpecOf(recorded: Encoders) =
    PcmSpec(MediaRecorder.AudioSource.CAMCORDER, recorded.sampleRate, if (recorded.channels == 1) AudioFormat.CHANNEL_IN_MONO else AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT)

  private fun outputFailed(error: Throwable) {
    Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, error)
    sink(CaptureEvent.Fault(FaultKind.OTHER))
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
    val cases = listOfNotNull(preview, photo, analysis)
    if (cases.isNotEmpty()) provider.unbind(*cases.toTypedArray())
    output?.release()
    output = null
    encoders = null
    preview = null
    photo = null
    analysis = null
    camera = null
  }

  override fun unbind() {
    session.incrementAndGet()
    take.incrementAndGet()
    lastBind = null
    main.execute {
      recording.close()
      cameras?.let(::release)
      audio.keep(null)
      if (watchingStream) {
        view?.previewStreamState?.removeObserver(streamObserver)
        watchingStream = false
      }
    }
  }

  override fun unbound(done: () -> Unit) { main.execute(done) }

  override fun close() {
    calculations.shutdown()
    main.execute { audio.close() }
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
    beginRecording(file, profile, lens) { it() }
  }

  override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens, start: (() -> Unit) -> Unit) {
    val generation = session.get()
    val recordingGeneration = take.incrementAndGet()
    main.execute {
      if (generation != session.get() || recordingGeneration != take.get()) return@execute
      try {
        val encoder = checkNotNull(output) { "no video output is bound" }
        val recorded = checkNotNull(encoders) { "no encoder profile is bound" }
        val memory = checkNotNull(memory) { "no capture memory is admitted" }
        val audioGeneration = if (profile.audio) checkNotNull(audio.generation) { "the microphone is not live" } else 0L
        val rotation = camera?.cameraInfo?.sensorRotationDegrees ?: 0
        val upright = if (rotation % 180 == 0) Size(recorded.width, recorded.height) else Size(recorded.height, recorded.width)
        val frameBytes = upright.width.toLong() * upright.height * BYTES_PER_PIXEL
        val frames = framesInFlight(memory.total, upright.width, upright.height)
        val held = memory.available - frameBytes
        check(held > 0) { "the capture memory holds no room for the take" }
        val vertex = uprightVertex(rotation, profile.mirrorFrontVideo && lens == CameraLens.FRONT)
        val feed = object : CameraEncoderFeed {
          override fun attach(generation: Long, target: Surface) = encoder.attach(generation, target, upright, vertex, frames)

          override fun detach(generation: Long) = encoder.detach(generation)

          override fun encoded(generation: Long, ptsUs: Long) = encoder.encoded(generation, ptsUs)
        }
        MediaTrace.emit("camera") { "take audio=${profile.audio}" }
        recording.start(file, recorded.settings(profile.audio, upright, audioGeneration, held, profile.maxDurationMs.toLong()), feed, start, { generation != session.get() || recordingGeneration != take.get() }, recordingGeneration) { reconcileAnalysis() }
      } catch (error: Exception) {
        Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.RECORDER, error)
        sink(CaptureEvent.RecordFinished(com.noma4i.medialayer.RecordEnd.FAILED))
      }
    }
  }

  override fun endRecording() {
    main.execute { recording.stop() }
  }

  override fun endRecording(done: () -> Unit) {
    main.execute { recording.stop(done) }
  }

  override fun audioLost() {
    main.execute { recording.audioLost() }
  }

  override fun awaitRecordingStopped(done: () -> Unit) {
    main.execute { recording.awaitStopped(done) }
  }

  override fun recordedMs(): Long = recording.recordedMs()

  override fun setTorch(on: Boolean) {
    main.execute { camera?.cameraControl?.enableTorch(on) }
  }

  override fun setTorch(on: Boolean, done: (Throwable?) -> Unit) {
    main.execute {
      val future = camera?.cameraControl?.enableTorch(on) ?: return@execute done(IllegalStateException("Camera is closed"))
      future.addListener({ try { future.get(); done(null) } catch (error: Exception) { done(error.cause ?: error) } }, main)
    }
  }

  override fun setZoom(ratio: Float) {
    main.execute { camera?.cameraControl?.setZoomRatio(ratio) }
  }

  override fun setZoom(ratio: Float, done: (Throwable?) -> Unit) {
    main.execute {
      val future = camera?.cameraControl?.setZoomRatio(ratio) ?: return@execute done(IllegalStateException("Camera is closed"))
      future.addListener({ try { future.get(); done(null) } catch (error: Exception) { done(error.cause ?: error) } }, main)
    }
  }

  override fun setFrameTap(analyzer: ImageAnalysis.Analyzer?) {
    main.execute {
      if (this.analyzer === analyzer) return@execute
      this.analyzer = analyzer
      if (analyzer != null && analysis != null) analysis?.setAnalyzer(main, analyzer)
      reconcileAnalysis()
    }
  }

  private fun reconcileAnalysis() {
    if (recording.active || camera == null || (analysis != null) == (analyzer != null)) return
    lastBind?.let { (lens, profile) -> bind(lens, profile) }
  }

  private fun plan(info: CameraInfo, profile: CaptureProfile): Result<Plan> {
    val camera2 = Camera2CameraInfo.from(info)
    val id = camera2.cameraId
    val map = camera2.getCameraCharacteristic(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return refused(PlanRefusal.NO_STREAM_MAP, PlanFacts(id, null, emptyList(), null, null, null))
    val jpeg = map.getOutputSizes(ImageFormat.JPEG).orEmpty().map { PixelSize(it.width.toUInt(), it.height.toUInt()) }
    val rates = camera2.getCameraCharacteristic(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty().map { FpsRange(it.lower.toUInt(), it.upper.toUInt()) }
    val supported = Recorder.getVideoCapabilities(info).getSupportedQualities(DynamicRange.SDR)
    val formats = QUALITIES.filter { it in supported }.mapNotNull { quality ->
      QualitySelector.getResolution(info, quality)?.let { quality to PixelSize(it.width.toUInt(), it.height.toUInt()) }
    }
    val offered = supported.map { it.toString() }
    val choice = pickFormat(formatCandidates(formats.map { it.second }, jpeg, rates), profile) ?: return refused(PlanRefusal.NO_FORMAT, PlanFacts(id, null, offered, null, null, null))
    val quality = formats[choice.index.toInt()].first
    val timebase = videoTimebaseOf(camera2.getCameraCharacteristic(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE))
    val encoders = if (profile.mode == CaptureMode.VIDEO) encodersOf(info, PlanFacts(id, quality.toString(), offered, null, null, null), quality, profile.audio, timebase).getOrElse { return Result.failure(it) } else null
    return Result.success(Plan(quality, choice.photoSize, choice.frameRate, encoders))
  }

  private fun <T> refused(reason: PlanRefusal, facts: PlanFacts): Result<T> = Result.failure(PlanRefused(reason, facts))

  /** The encoder profile comes from the provider of the CameraX camera, the one the CameraX recorder reads: it holds the profile of a camera the legacy `CamcorderProfile` has none for. */
  @android.annotation.SuppressLint("RestrictedApi")
  private fun encodersOf(info: CameraInfo, facts: PlanFacts, quality: Quality, audio: Boolean, timebase: VideoTimebase): Result<Encoders> {
    val all = (info as CameraInfoInternal).encoderProfilesProvider.getAll(CAMCORDER_QUALITIES.getValue(quality))
    val counted = PlanFacts(facts.cameraId, facts.quality, facts.supported, all?.videoProfiles?.size ?: 0, all?.audioProfiles?.size ?: 0, null)
    val made = encodersFrom(all, audio, timebase) ?: return refused(PlanRefusal.NO_SDR_PROFILE, counted)
    if (!audio) return Result.success(made)
    val supported = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { it.isEncoder && it.supportedTypes.contains(MediaFormat.MIMETYPE_AUDIO_AAC) }
      .flatMap { it.getCapabilitiesForType(MediaFormat.MIMETYPE_AUDIO_AAC).profileLevels.map { level -> level.profile } }.toSet()
    val chosen = resolveAacProfile(made.audioProfile, supported)?.let { made.withAudioProfile(it) } ?: made
    if (resolveAacProfile(made.audioProfile, supported) == null || MediaCodecList(MediaCodecList.REGULAR_CODECS).findEncoderForFormat(checkNotNull(chosen.audioFormat())) == null) return refused(PlanRefusal.NO_AUDIO_ENCODER, PlanFacts(counted.cameraId, counted.quality, counted.supported, counted.videoProfiles, counted.audioProfiles, chosen))
    return Result.success(chosen)
  }

  private fun aspect(size: PixelSize): AspectRatioStrategy {
    val wide = size.width.toDouble() / size.height.toDouble()
    val ratio = if (wide >= 1) wide else 1 / wide
    return if (kotlin.math.abs(ratio - 16.0 / 9) / (16.0 / 9) < ASPECT_TOLERANCE) AspectRatioStrategy.RATIO_16_9_FALLBACK_AUTO_STRATEGY else AspectRatioStrategy.RATIO_4_3_FALLBACK_AUTO_STRATEGY
  }

  private fun previewSelector(plan: Plan?, profile: CaptureProfile, recorded: Encoders?): ResolutionSelector {
    val builder = ResolutionSelector.Builder()
    recorded?.let { video -> builder.setResolutionFilter { supported, _ -> supported.filter { it.width <= video.width && it.height <= video.height }.ifEmpty { supported } } }
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

  private fun isRecoverable(code: Int) = code == CameraState.ERROR_CAMERA_IN_USE || code == CameraState.ERROR_MAX_CAMERAS_IN_USE || code == CameraState.ERROR_OTHER_RECOVERABLE_ERROR

  private companion object {
    const val AE_MODE_ON_AUTO_FLASH_REDEYE = 4
    const val ASPECT_TOLERANCE = 0.02
    const val BYTES_PER_PIXEL = 4L
    val QUALITIES = listOf(Quality.SD, Quality.HD, Quality.FHD, Quality.UHD)
    val CAMCORDER_QUALITIES = mapOf(Quality.SD to CamcorderProfile.QUALITY_480P, Quality.HD to CamcorderProfile.QUALITY_720P, Quality.FHD to CamcorderProfile.QUALITY_1080P, Quality.UHD to CamcorderProfile.QUALITY_2160P)
  }
}

package com.noma4i.medialayer.ui

import android.app.Application
import android.graphics.Bitmap
import android.os.Looper
import androidx.camera.core.ImageAnalysis
import androidx.camera.view.PreviewView
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.lifecycle.LifecycleOwner
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CameraRefusal
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.FaultKind
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.RecordEnd
import com.noma4i.medialayer.VideoStabilization
import com.noma4i.medialayer.cameraRefusalText
import com.noma4i.medialayer.captureProfile
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private class FakeBackend : CaptureBackend {
  val log = CopyOnWriteArrayList<String>()
  val mainCalls = AtomicInteger()
  @Volatile private var sink: (CaptureEvent) -> Unit = {}
  @Volatile var tap: ImageAnalysis.Analyzer? = null
  @Volatile var recordingFile: File? = null
  @Volatile var photoFails = false
  @Volatile var hasFlash = true
  @Volatile var recordedProfile: CaptureProfile? = null
  val recordedReads = AtomicInteger()
  override val photoSize: PixelSize? = PixelSize(3000u, 4000u)

  fun emit(event: CaptureEvent) = sink(event)

  fun count(entry: String) = log.count { it == entry }

  private fun note(entry: String) {
    log.add(entry)
    if (Looper.myLooper() == Looper.getMainLooper()) mainCalls.incrementAndGet()
    TestSignal.fire()
  }

  override fun setSink(sink: (CaptureEvent) -> Unit) {
    this.sink = sink
  }

  override fun setHost(view: PreviewView?, owner: LifecycleOwner?) = Unit

  override fun bind(lens: CameraLens, profile: CaptureProfile) {
    note("bind $lens")
    emit(CaptureEvent.Opened(hasFlash, 1f, 20f))
    emit(CaptureEvent.Streaming(true))
  }

  override fun unbind() = note("unbind")

  override suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile) {
    note("photo flash=$flash")
    if (photoFails) throw IllegalStateException("fake")
    file.writeBytes(byteArrayOf(1, 2, 3))
  }

  override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens) {
    note("record $lens")
    recordedProfile = profile
    recordingFile = file
    file.writeBytes(byteArrayOf(1, 2, 3))
    emit(CaptureEvent.RecordStarted)
  }

  override fun endRecording() {
    note("endRecording")
    emit(CaptureEvent.RecordFinished(RecordEnd.COMPLETED))
  }

  override fun recordedMs(): Long {
    recordedReads.incrementAndGet()
    return 700
  }

  override fun setTorch(on: Boolean) = note("torch $on")

  override fun setZoom(ratio: Float) = note("zoom $ratio")

  override fun setFrameTap(analyzer: ImageAnalysis.Analyzer?) {
    tap = analyzer
    TestSignal.fire()
  }

  override fun sensorToView(): android.graphics.Matrix? = null
}

private class Counters {
  val taken = AtomicInteger()
  val released = AtomicInteger()
  val qualities = CopyOnWriteArrayList<Double>()
  val mainCalls = AtomicInteger()
  @Volatile var denySessions = false
  @Volatile var denyShots = false
  @Volatile var info: MediaInfo? = MediaInfo(2000UL, ImageSize(720u, 1280u))
  @Volatile var videoGate: CompletableDeferred<Unit>? = null
  val straightened = CopyOnWriteArrayList<File>()
  val straightenCancelled = java.util.concurrent.atomic.AtomicBoolean()

  fun checkThread() {
    if (Looper.myLooper() == Looper.getMainLooper()) mainCalls.incrementAndGet()
  }
}

private class Rig(val camera: MediaCamera, val backend: FakeBackend, val counters: Counters, val directory: File, val audio: MediaAudioSession) {
  val host = Any()

  fun attach() = camera.attachPreview(host, null, null)

  fun detach() = camera.detachPreview(host)

  fun files(): List<String> = (directory.list() ?: emptyArray()).sorted()
}

private fun eventually(condition: String, holds: () -> Boolean) {
  TestSignal.await(condition, 5_000, holds)
}

@RunWith(MediaLayerTestRunner::class)
@OptIn(ExperimentalCoroutinesApi::class)
class MediaCameraTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()

  @get:Rule
  val compose = createComposeRule()

  private fun rig(profile: CaptureProfile = captureProfile(CaptureKind.PHOTO), lens: CameraLens = CameraLens.BACK, scope: CoroutineScope = CoroutineScope(Dispatchers.Default), configure: (FakeBackend, Counters) -> Unit = { _, _ -> }): Rig {
    val backend = FakeBackend()
    val counters = Counters()
    configure(backend, counters)
    val directory = File(context.cacheDir, "camera-${System.nanoTime()}")
    val audio = MediaAudioSession(MediaAudioFocus.Mix, FakeFocusPort(), releaseDelayMs = 0)
    val services = CameraServices(
      admitSession = { _ ->
        counters.checkThread()
        if (counters.denySessions) throw MediaException.Overloaded("no memory")
        counters.taken.incrementAndGet()
        AutoCloseable { counters.released.incrementAndGet(); TestSignal.fire() }
      },
      admitShot = { _ ->
        counters.checkThread()
        if (counters.denyShots) throw MediaException.Overloaded("no memory")
        counters.taken.incrementAndGet()
        AutoCloseable { counters.released.incrementAndGet(); TestSignal.fire() }
      },
      audio = { audio },
      clipInfo = { _ ->
        counters.checkThread()
        counters.info
      },
      uprightVideo = { file ->
        counters.straightened.add(file)
        TestSignal.fire()
        try {
          counters.videoGate?.await()
        } catch (cancel: kotlinx.coroutines.CancellationException) {
          counters.straightenCancelled.set(true)
          throw cancel
        }
      },
      upright = { raw, quality ->
        counters.checkThread()
        counters.qualities.add(quality)
        val made = File(raw.parentFile, "made-${System.nanoTime()}.jpg")
        made.outputStream().use { Bitmap.createBitmap(3, 5, Bitmap.Config.ARGB_8888).compress(Bitmap.CompressFormat.JPEG, 90, it) }
        made
      },
      directory = directory
    )
    return Rig(MediaCamera(profile, lens, backend, services, scope), backend, counters, directory, audio)
  }

  private fun ready(rig: Rig) {
    rig.attach()
    eventually("ready") { rig.camera.state.value.phase == CameraPhase.READY }
  }

  private val moment get() = captureProfile(CaptureKind.MOMENT)

  @Test
  fun inv_capture_13_theSessionTakesItsMemoryBeforeTheBindAndGivesItBackWhenThePreviewLeaves() {
    val rig = rig()
    ready(rig)
    assertEquals(1 to 0, rig.counters.taken.get() to rig.counters.released.get())
    rig.detach()
    eventually("released") { rig.counters.released.get() == 1 }
    assertEquals(1, rig.backend.count("unbind"))
  }

  @Test
  fun inv_capture_13_aRefusalOfTheMemoryBudgetFailsTheCameraWithoutABind() {
    val rig = rig { _, counters -> counters.denySessions = true }
    rig.attach()
    eventually("failed") { rig.camera.state.value.phase == CameraPhase.FAILED }
    assertEquals(CameraRefusal.MEMORY, rig.camera.state.value.fault)
    assertTrue(rig.backend.log.none { it.startsWith("bind") })
  }

  @Test
  fun inv_capture_13_theMemoryOfAShotIsGivenBackAfterAPhotoAndAfterAFailedPhoto() = runBlocking {
    val rig = rig()
    ready(rig)
    rig.camera.takePhoto()
    assertEquals(2 to 1, rig.counters.taken.get() to rig.counters.released.get())
    rig.backend.photoFails = true
    assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.takePhoto() } }
    assertEquals(3 to 2, rig.counters.taken.get() to rig.counters.released.get())
    assertEquals(CameraRefusal.PHOTO_FAILED, rig.camera.state.value.fault)
    rig.counters.denyShots = true
    val refused = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.takePhoto() } }
    assertEquals(CameraRefusal.MEMORY, refused.reason)
  }

  @Test
  fun inv_capture_15_aPhotoIsAnUprightJpegFileWithItsSizeAndTheLensAndTheRawFileIsGone() = runBlocking {
    val rig = rig(captureProfile(CaptureKind.PHOTO), CameraLens.FRONT)
    ready(rig)
    val photo = rig.camera.takePhoto()
    assertEquals(Triple(3, 5, CameraLens.FRONT), Triple(photo.width, photo.height, photo.lens))
    assertTrue(photo.file.name.startsWith("capture-") && photo.file.extension == "jpg" && photo.file.isFile)
    assertEquals(listOf(0.9), rig.counters.qualities.toList())
    assertEquals(listOf(photo.file.name), rig.files())
    assertFalse(rig.camera.state.value.shooting)
  }

  @Test
  fun inv_capture_14_aThrownAwayShortAndFailedRecordingLeavesNoFileAndADeliveredOneStays() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    val delivered = rig.camera.stopRecording()
    assertEquals(Triple(2000L, 720u, CameraLens.BACK), Triple(delivered.durationMs, delivered.frame?.width, delivered.lens))
    assertEquals(listOf(delivered.file.name), rig.files())
    eventually("ready") { rig.camera.state.value.phase == CameraPhase.READY }

    rig.counters.info = MediaInfo(300UL, null)
    rig.camera.startRecording()
    val tooShort = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    assertEquals(CameraRefusal.TOO_SHORT to cameraRefusalText(CameraRefusal.TOO_SHORT), tooShort.reason to tooShort.detail)
    assertEquals(listOf(delivered.file.name), rig.files())

    rig.counters.info = null
    eventually("ready again") { rig.camera.state.value.phase == CameraPhase.READY }
    rig.camera.startRecording()
    assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    assertEquals(CameraRefusal.RECORD_FAILED, rig.camera.state.value.fault)
    assertEquals(listOf(delivered.file.name), rig.files())

    eventually("ready third") { rig.camera.state.value.phase == CameraPhase.READY }
    rig.camera.startRecording()
    rig.camera.cancelRecording()
    eventually("ready after cancel") { rig.camera.state.value.phase == CameraPhase.READY }
    assertEquals(listOf(delivered.file.name), rig.files())
  }

  @Test
  fun inv_call_13_aCallThrowsARunningVideoRecordingAwayWithTheInCallRefusalAndRefusesANewOne() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()

    Media.calls.hold()
    try {
      val ended = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
      assertEquals(CameraRefusal.IN_CALL to cameraRefusalText(CameraRefusal.IN_CALL), ended.reason to ended.detail)
      assertTrue(rig.files().isEmpty())
      eventually("ready after the call") { rig.camera.state.value.phase == CameraPhase.READY }

      val refused = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.startRecording() } }
      assertEquals(CameraRefusal.IN_CALL, refused.reason)
    } finally {
      Media.calls.release()
    }
  }

  @Test
  fun inv_capture_14_theOrphansOfAnEarlierRunAreRemovedAndOtherFilesStay() {
    val directory = File(context.cacheDir, "sweep-${System.nanoTime()}").apply { mkdirs() }
    listOf("capture-a.jpg", "capture-b.mp4", "keep.txt", "voice-a.m4a").forEach { File(directory, it).writeBytes(byteArrayOf(1)) }
    CaptureFiles.sweepOrphans(directory)
    assertEquals(listOf("keep.txt", "voice-a.m4a"), directory.list()!!.sorted())
  }

  @Test
  fun inv_capture_16_commandsFromTheMainThreadRunNoHardwareCoreOrFileWorkOnIt() = runBlocking {
    val rig = rig(moment)
    rig.attach()
    eventually("ready") { rig.camera.state.value.phase == CameraPhase.READY }
    rig.camera.setFlash(true)
    rig.camera.zoomBy(2.0)
    rig.camera.setLens(CameraLens.FRONT)
    eventually("front ready") { rig.camera.state.value.lens == CameraLens.FRONT && rig.camera.state.value.phase == CameraPhase.READY }
    rig.camera.startRecording()
    rig.camera.stopRecording()
    rig.detach()
    eventually("unbound") { rig.backend.count("unbind") == 1 }
    assertEquals(0, rig.backend.mainCalls.get())
    assertEquals(0, rig.counters.mainCalls.get())
  }

  @Test
  fun inv_capture_17_thePreviewLeavingReleasesTheCameraAndTheReturnBindsAgain() {
    val rig = rig()
    ready(rig)
    val second = Any()
    rig.camera.attachPreview(second, null, null)
    rig.camera.detachPreview(second)
    Thread.sleep(50)
    assertEquals(1 to 0, rig.backend.count("bind BACK") to rig.backend.count("unbind"))
    rig.detach()
    eventually("released") { rig.backend.count("unbind") == 1 }
    assertEquals(CameraPhase.IDLE, rig.camera.state.value.phase)
    rig.attach()
    eventually("bound again") { rig.backend.count("bind BACK") == 2 && rig.camera.state.value.phase == CameraPhase.READY }
  }

  @Test
  fun inv_capture_17_theCameraFollowsTheCompositionOfThePreview() {
    val rig = rig()
    var shown by mutableStateOf(true)
    compose.setContent {
      val context = LocalContext.current
      if (shown) androidx.compose.runtime.DisposableEffect(rig.camera) {
        rig.camera.attachPreview(this, cameraPreviewView(context), null)
        onDispose { rig.camera.detachPreview(this) }
      }
    }
    eventually("bound") { rig.backend.count("bind BACK") == 1 }
    shown = false
    compose.waitForIdle()
    eventually("unbound") { rig.backend.count("unbind") == 1 }
  }

  @Test
  fun inv_capture_23_thePreviewViewFillsTheFrame() {
    val view = cameraPreviewView(context)
    assertEquals(PreviewView.ScaleType.FILL_CENTER, view.scaleType)
    assertEquals(PreviewView.ImplementationMode.COMPATIBLE, view.implementationMode)
  }

  @Test
  fun inv_capture_18_anIdleCameraHoldsNoSessionNoTickerAndNoFrameOutput() = runTest {
    val rig = rig(moment, scope = backgroundScope)
    advanceTimeBy(10_000)
    runCurrent()
    assertTrue(rig.backend.log.isEmpty() && rig.backend.tap == null && rig.counters.taken.get() == 0)
    rig.attach()
    rig.camera.awaitQueue()
    runCurrent()
    rig.camera.state.first { it.phase == CameraPhase.READY }
    rig.camera.startRecording()
    rig.camera.awaitQueue()
    runCurrent()
    advanceTimeBy(1_000)
    runCurrent()
    assertTrue(rig.backend.recordedReads.get() > 0)
    rig.camera.stopRecording()
    rig.camera.awaitQueue()
    val reads = rig.backend.recordedReads.get()
    advanceTimeBy(10_000)
    runCurrent()
    assertEquals("the ticker stops with the recording", reads, rig.backend.recordedReads.get())
  }

  @Test
  fun inv_capture_19_aResetRestartsOnceAndTheSecondOneFailsUntilThePreviewReturns() {
    val rig = rig()
    ready(rig)
    rig.backend.emit(CaptureEvent.Fault(FaultKind.RESET))
    eventually("rebound") { rig.backend.count("bind BACK") == 2 && rig.camera.state.value.phase == CameraPhase.READY }
    assertEquals(1, rig.backend.count("unbind"))
    rig.backend.emit(CaptureEvent.Fault(FaultKind.RESET))
    eventually("failed") { rig.camera.state.value.phase == CameraPhase.FAILED }
    assertEquals(CameraRefusal.BUSY, rig.camera.state.value.fault)
    rig.detach()
    rig.attach()
    eventually("ready again") { rig.camera.state.value.phase == CameraPhase.READY }
    rig.backend.emit(CaptureEvent.Fault(FaultKind.RESET))
    eventually("restarted again") { rig.backend.count("bind BACK") >= 4 && rig.camera.state.value.phase == CameraPhase.READY }
  }

  @Test
  fun inv_capture_20_aRecordingStartsOnTheEventOfTheHardwareEndsByTheLimitAndKeepsItsClip() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    assertEquals(CameraPhase.RECORDING, rig.camera.state.value.phase)
    eventually("length published") { rig.camera.state.value.recordedMs == 700UL }
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    eventually("settled") { rig.camera.state.value.phase == CameraPhase.READY }
    val clip = rig.camera.stopRecording()
    assertEquals(2000L to "mp4", clip.durationMs to clip.file.extension)
    assertTrue(clip.file.isFile)
  }

  @Test
  fun inv_capture_20_aLensChangeDuringARecordingThrowsItAwayAndBindsTheOtherSide() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    val recorded = rig.backend.recordingFile!!
    rig.camera.setLens(CameraLens.FRONT)
    eventually("front bound") { rig.backend.count("bind FRONT") == 1 && rig.camera.state.value.phase == CameraPhase.READY }
    assertEquals(CameraLens.FRONT, rig.camera.state.value.lens)
    assertFalse(recorded.exists())
    assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    Unit
  }

  @Test
  fun inv_capture_20_thePreviewLeavingDuringARecordingThrowsTheClipAwayAndFreesTheCameraAndTheMicrophone() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    val recorded = rig.backend.recordingFile!!
    val stopping = async(Dispatchers.Default) { runCatching { rig.camera.stopRecording() } }
    rig.detach()
    eventually("unbound") { rig.backend.count("unbind") == 1 && rig.counters.released.get() == 1 }
    assertFalse(recorded.exists())
    assertTrue(stopping.await().isFailure)
    assertEquals(CameraPhase.IDLE to 0UL, rig.camera.state.value.phase to rig.camera.state.value.recordedMs)
    eventually("microphone given back") { !rig.audio.isRecording }
  }

  @Test
  fun inv_capture_21_aProfileWithSoundTakesTheRecordingFocusOnOpenAndGivesItBackOnRelease() {
    val rig = rig(moment)
    assertFalse(rig.audio.isRecording)
    ready(rig)
    assertTrue(rig.audio.isRecording)
    rig.detach()
    eventually("given back") { !rig.audio.isRecording }
    val still = rig()
    ready(still)
    assertFalse("a photo profile does not take the microphone", still.audio.isRecording)
  }

  @Test
  fun inv_capture_22_theFrameOutputStandsOnlyWhileSomebodyListens() {
    val rig = rig()
    ready(rig)
    assertNull(rig.backend.tap)
    val listening = rig.camera.observeFrames(10.0, ImageAnalysis.Analyzer { it.close() })
    eventually("tap installed") { rig.backend.tap != null }
    listening.close()
    eventually("tap removed") { rig.backend.tap == null }
  }

  @Test
  fun inv_capture_24_aClosedCameraReleasesTheSessionAndRefusesCommands() = runBlocking {
    val rig = rig()
    ready(rig)
    rig.camera.close()
    eventually("unbound by close") { rig.backend.count("unbind") == 1 && rig.counters.released.get() == 1 }
    assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.takePhoto() } }
    Unit
  }

  @Test
  fun inv_capture_25_aRecordingHandsTheStabilizationOfItsProfileToTheBackend() = runBlocking {
    for ((profile, expected) in listOf(captureProfile(CaptureKind.CIRCLE) to VideoStabilization.CINEMATIC, moment to VideoStabilization.OFF)) {
      val rig = rig(profile)
      ready(rig)
      rig.camera.startRecording()
      assertEquals(expected, rig.backend.recordedProfile?.stabilization)
    }
  }

  @Test
  fun inv_capture_27_aCircleProfileWithALeastTakeOf500MsIsBuiltAndReachesTheBackend() = runBlocking {
    val rig = rig(captureProfile(CaptureKind.CIRCLE).copy(minTakeMs = 500UL))
    ready(rig)
    rig.camera.startRecording()
    assertEquals(500UL, rig.backend.recordedProfile?.minTakeMs)
    assertEquals(VideoStabilization.CINEMATIC, rig.backend.recordedProfile?.stabilization)
  }

  @Test
  fun inv_capture_28_aTooShortRecordingCarriesItsLengthInTheError() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.counters.info = MediaInfo(400UL, null)
    rig.camera.startRecording()
    val tooShort = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    assertEquals(CameraRefusal.TOO_SHORT to 400UL, tooShort.reason to tooShort.recordedMs)

    rig.counters.info = null
    eventually("ready again") { rig.camera.state.value.phase == CameraPhase.READY }
    rig.camera.startRecording()
    val failed = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    assertEquals(CameraRefusal.RECORD_FAILED to null, failed.reason to failed.recordedMs)
  }

  @Test
  fun inv_capture_09_aBadProfileMakesNoCamera() {
    val broken = captureProfile(CaptureKind.MOMENT).copy(videoSize = null)
    assertThrows(MediaException::class.java) { MediaCamera(context, broken) }
  }

  @Test
  fun inv_capture_29_aFinishedTakeIsStraightenedWhileStoppingAndACancelThenThrowsItAway() = runBlocking {
    val gate = CompletableDeferred<Unit>()
    val rig = rig(moment) { _, counters -> counters.videoGate = gate }
    ready(rig)
    rig.camera.startRecording()
    val recorded = rig.backend.recordingFile!!
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    eventually("straightening while stopping") { rig.counters.straightened.toList() == listOf(recorded) && rig.camera.state.value.phase == CameraPhase.STOPPING }
    rig.camera.cancelRecording()
    eventually("ready after the cancel") { rig.camera.state.value.phase == CameraPhase.READY }
    assertTrue("the conversion was cancelled", rig.counters.straightenCancelled.get())
    assertFalse(recorded.exists())
    val refused = runCatching { rig.camera.stopRecording() }.exceptionOrNull()
    assertTrue("$refused", refused is MediaException.Camera)
  }

  @Test
  fun inv_capture_31_theClipOfATakeIsGivenOnce() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    val recorded = rig.backend.recordingFile!!
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    eventually("settled") { rig.camera.state.value.phase == CameraPhase.READY }
    val clip = rig.camera.stopRecording()
    assertTrue(clip.file.isFile)
    assertEquals(listOf(recorded), rig.counters.straightened.toList())
    val again = runCatching { rig.camera.stopRecording() }.exceptionOrNull()
    assertEquals(CameraRefusal.NOT_READY, (again as? MediaException.Camera)?.reason)
  }
}

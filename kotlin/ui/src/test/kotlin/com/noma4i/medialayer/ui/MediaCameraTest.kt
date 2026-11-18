package com.noma4i.medialayer.ui

import com.noma4i.medialayer.checkCaptureProfile
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
import com.noma4i.medialayer.TakeOutcome
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.FailureSource
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
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
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.After
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private class FakeBackend : CaptureBackend {
  val log = CopyOnWriteArrayList<String>()
  val mainCalls = AtomicInteger()
  @Volatile private var sink: (CaptureEvent) -> Unit = {}
  @Volatile private var audioSink: (CaptureAudioIssue) -> Unit = {}
  @Volatile var tap: ImageAnalysis.Analyzer? = null
  @Volatile var recordingFile: File? = null
  @Volatile var photoFails = false
  @Volatile var photoGate: java.util.concurrent.CountDownLatch? = null
  @Volatile var hasFlash = true
  @Volatile var recordedProfile: CaptureProfile? = null
  @Volatile var torchFailure: Exception? = null
  @Volatile var zoomFailure: Exception? = null
  val recordedReads = AtomicInteger()
  override val photoSize: PixelSize? = PixelSize(3000u, 4000u)

  fun emit(event: CaptureEvent) = sink(event)

  /** The microphone sound reported after a take begins, as CameraX AudioStats do; C4 turns it off to hold it. */
  var soundOnRecord = true

  fun audioIssue(issue: CaptureAudioIssue) = audioSink(issue)

  fun count(entry: String) = log.count { it == entry }

  private fun note(entry: String) {
    log.add(entry)
    if (Looper.myLooper() == Looper.getMainLooper()) mainCalls.incrementAndGet()
    TestSignal.fire()
  }

  override fun setSink(sink: (CaptureEvent) -> Unit) {
    this.sink = sink
  }

  override fun setAudioIssueSink(sink: (CaptureAudioIssue) -> Unit) { audioSink = sink }

  override fun setHost(view: PreviewView?, owner: LifecycleOwner?) = Unit

  @Volatile var memory: CaptureMemory? = null

  override fun useMemory(memory: CaptureMemory) { this.memory = memory }

  override fun bind(lens: CameraLens, profile: CaptureProfile) {
    note("bind $lens")
    emit(CaptureEvent.Opened(hasFlash, 1f, 20f))
    emit(CaptureEvent.Streaming(true))
  }

  override fun unbind() = note("unbind")

  override suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile) {
    note("photo flash=$flash")
    photoGate?.await()
    if (photoFails) throw IllegalStateException("fake")
    file.writeBytes(byteArrayOf(1, 2, 3))
  }

  override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens) {
    note("record $lens")
    recordedProfile = profile
    recordingFile = file
    file.writeBytes(byteArrayOf(1, 2, 3))
    emit(CaptureEvent.RecordStarted)
    if (soundOnRecord && profile.audio) audioIssue(CaptureAudioIssue.ACTIVE)
  }

  override fun endRecording() {
    note("endRecording")
    emit(CaptureEvent.RecordFinished(RecordEnd.COMPLETED))
  }

  /** While set, a stop of the take waits for [finalizeHeldStops], as CameraX waits for its Finalize event. */
  @Volatile var holdStops = false
  val heldStops = CopyOnWriteArrayList<() -> Unit>()

  override fun endRecording(done: () -> Unit) {
    if (!holdStops) return super.endRecording(done)
    heldStops.add { emit(CaptureEvent.RecordFinished(RecordEnd.COMPLETED)); done() }
    note("endRecording held")
  }

  override fun awaitRecordingStopped(done: () -> Unit) {
    if (!holdStops) return done()
    heldStops.add(done)
    note("stop held")
  }

  fun finalizeHeldStops() {
    holdStops = false
    heldStops.toList().also { heldStops.clear() }.forEach { it() }
  }

  override fun recordedMs(): Long {
    recordedReads.incrementAndGet()
    return 700
  }

  override fun setTorch(on: Boolean) {
    note("torch $on")
    torchFailure?.let { throw it }
  }

  override fun setZoom(ratio: Float) {
    note("zoom $ratio")
    zoomFailure?.let { throw it }
  }

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
  @Volatile var coverage = CoreCoverage.covered()
  val coverageSeen = CopyOnWriteArrayList<File>()
  @Volatile var videoGate: CompletableDeferred<Unit>? = null
  @Volatile var focusAnswer = com.noma4i.medialayer.FocusAnswer.GRANTED
  val straightened = CopyOnWriteArrayList<File>()
  val straightenCancelled = java.util.concurrent.atomic.AtomicBoolean()

  fun checkThread() {
    if (Looper.myLooper() == Looper.getMainLooper()) mainCalls.incrementAndGet()
  }
}

private class Rig(val camera: MediaCamera, val backend: FakeBackend, val counters: Counters, val directory: File, val fixture: AudioFixture) {
  val audio: AudioLink get() = fixture.link
  val host = Any()

  fun attach() = camera.attachPreview(host, null, null)

  fun detach() = camera.detachPreview(host)

  fun files(): List<String> = (directory.list() ?: emptyArray()).sorted()
}

private fun eventually(condition: String, holds: () -> Boolean) {
  TestSignal.await(condition, 5_000, holds)
}

@RunWith(MediaLayerTestRunner::class)
@OptIn(ExperimentalCoroutinesApi::class, MediaLayerTestingApi::class)
class MediaCameraTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()
  private val fixtures = mutableListOf<AudioFixture>()
  @After fun closeAudio() { fixtures.forEach { it.close() } }

  @get:Rule
  val compose = createComposeRule()

  private fun rig(profile: CaptureProfile = captureProfile(CaptureKind.PHOTO), lens: CameraLens = CameraLens.BACK, scope: CoroutineScope = CoroutineScope(Dispatchers.Default), app: AppPath? = null, screen: ScreenSwitch = ScreenSwitch(), configure: (FakeBackend, Counters) -> Unit = { _, _ -> }): Rig {
    val backend = FakeBackend()
    val counters = Counters()
    configure(backend, counters)
    val directory = File(context.cacheDir, "camera-${System.nanoTime()}")
    val fixture = AudioFixture(context).also { fixtures.add(it) }
    fixture.port.answer = counters.focusAnswer
    val audio: () -> AudioLink? = if (app != null) { { Media.link } } else { { fixture.link } }
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
      audio = audio,
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
      directory = directory,
      audioCoverage = { file -> counters.coverageSeen.add(file); counters.coverage }
    )
    return Rig(MediaCamera(profile, lens, backend, services, scope, screen = screen.awake()), backend, counters, directory, fixture)
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
  fun inv_capture_13_theLedgerHoldStaysUntilTheBackendReturnedEveryLeaseAndARebindKeepsTheSameTotal() {
    val rig = rig()
    ready(rig)
    val memory = rig.backend.memory!!
    val lease = memory.lease(1024)
    rig.camera.setLens(CameraLens.FRONT)
    eventually("rebound") { rig.backend.count("bind FRONT") == 1 }
    assertTrue("a rebind keeps the one total", rig.backend.memory === memory && rig.counters.taken.get() == 1)
    rig.detach()
    eventually("unbound") { rig.backend.count("unbind") == 1 }
    runBlocking { rig.camera.awaitQueue() }
    assertEquals("a lease still lives: the Ledger hold stays", 0, rig.counters.released.get())
    lease.close()
    eventually("the hold is given back after the last lease") { rig.counters.released.get() == 1 }
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
  fun inv_api_63_a_camera_preview_on_screen_keeps_the_screen_on() = runBlocking {
    val screen = ScreenSwitch()
    val rig = rig(moment, screen = screen)
    assertFalse(screen.on)

    ready(rig)
    eventually("the screen stays on while the capture screen shows the preview") { screen.on }

    rig.camera.startRecording()
    rig.camera.stopRecording()
    assertTrue("a recording keeps the hold of its preview", screen.on)

    rig.detach()
    eventually("the screen may sleep after the preview leaves") { !screen.on }
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

    rig.fixture.port.send(FocusSignal.PhoneCall(true))
    eventually("call revokes recording") { !rig.audio.isRecording }
    try {
      val ended = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
      assertEquals(CameraRefusal.IN_CALL to cameraRefusalText(CameraRefusal.IN_CALL), ended.reason to ended.detail)
      assertTrue(rig.files().isEmpty())
      eventually("ready after the call") { rig.camera.state.value.phase == CameraPhase.READY }

      val refused = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.startRecording() } }
      assertEquals(CameraRefusal.IN_CALL, refused.reason)
    } finally {
      rig.fixture.port.send(FocusSignal.PhoneCall(false))
    }
  }

  @Test
  fun inv_audio_80_aDrivenSystemEventThrowsACameraTakeAwayWithItsRefusal() = runBlocking {
    for ((signal, expected) in listOf(
      AudioDriveSignal.StartCall to CameraRefusal.IN_CALL,
      AudioDriveSignal.SilenceMicrophone to CameraRefusal.MICROPHONE_SILENT,
      AudioDriveSignal.BeginInterruption to CameraRefusal.INTERRUPTED
    )) {
      val rig = rig(moment)
      ready(rig)
      rig.camera.startRecording()

      rig.audio.drive(signal)

      eventually("the take ended by $signal") { rig.camera.state.value.phase != CameraPhase.RECORDING }
      val ended = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
      assertEquals(signal.toString(), expected, ended.reason)
      assertTrue(rig.files().isEmpty())
    }
  }

  @Test
  fun inv_audio_43_aTakeThatLosesItsMicrophoneSoundIsThrownAwayWithTheSilentRefusal() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()

    rig.backend.audioIssue(CaptureAudioIssue.SILENCED)

    eventually("the take ended") { rig.camera.state.value.phase != CameraPhase.RECORDING }
    val ended = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    assertEquals(CameraRefusal.MICROPHONE_SILENT to cameraRefusalText(CameraRefusal.MICROPHONE_SILENT), ended.reason to ended.detail)
    assertTrue(rig.files().isEmpty())
  }

  @Test
  fun inv_audio_70_a_recorded_file_whose_sound_the_core_refuses_is_deleted_with_the_silent_refusal_and_one_outcome() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    val outcomes = CopyOnWriteArrayList<Any>()
    val watching = launch(Dispatchers.Default, start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { rig.camera.state.collect { state -> state.outcome?.let { outcomes.add(it) } } }
    CoreCoverage.refused.forEach { (verdict, answer) ->
      assertEquals(verdict, answer.verdict)
      rig.counters.coverage = answer
      rig.camera.startRecording()
      val ended = assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
      assertEquals(CameraRefusal.MICROPHONE_SILENT, ended.reason)
      assertTrue(rig.files().isEmpty())
      eventually("ready after $verdict") { rig.camera.state.value.phase == CameraPhase.READY }
    }
    watching.cancel()
    assertEquals(listOf<Any>(com.noma4i.medialayer.TakeOutcome.Refused(CameraRefusal.MICROPHONE_SILENT)), outcomes.distinct())
  }

  @Test
  fun inv_audio_71_the_sound_is_read_from_the_raw_file_before_the_upright_pass_and_again_after_it() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    val clip = rig.camera.stopRecording()
    assertEquals(listOf(clip.file, clip.file), rig.counters.coverageSeen.toList())
    assertEquals(listOf(clip.file), rig.counters.straightened.toList())
  }

  @Test
  fun inv_audio_72_a_take_with_no_audio_in_its_profile_skips_the_sound_read() = runBlocking {
    val rig = rig(moment.copy(audio = false))
    ready(rig)
    rig.counters.coverage = CoreCoverage.missing()
    rig.camera.startRecording()
    rig.camera.stopRecording()
    assertTrue(rig.counters.coverageSeen.isEmpty())
  }

  @Test
  fun inv_audio_73_aac_frame_samples_come_from_the_audio_specific_config_and_only_for_lc_at_the_output_rate() {
    val lc = java.nio.ByteBuffer.wrap(androidx.media3.extractor.AacUtil.buildAacLcAudioSpecificConfig(44_100, 1))
    assertEquals(1024, aacFrameSamples(lc, 44_100))
    assertEquals(0, aacFrameSamples(lc, 48_000))
    assertEquals(0, aacFrameSamples(java.nio.ByteBuffer.wrap(byteArrayOf(0x42, 0x08)), 44_100))
    assertEquals(0, aacFrameSamples(null, 44_100))
  }

  @Test
  fun inv_capture_33_after_a_silent_take_the_next_take_starts_because_camerax_reports_sound_only_during_a_take() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.startRecording()
    rig.backend.audioIssue(CaptureAudioIssue.SILENCED)
    eventually("the take ended") { rig.camera.state.value.phase != CameraPhase.RECORDING }
    assertThrows(MediaException.Camera::class.java) { runBlocking { rig.camera.stopRecording() } }
    val before = rig.backend.log.count { it.startsWith("record ") }

    rig.camera.startRecording()

    assertEquals("the next take began on the hardware", before + 1, rig.backend.log.count { it.startsWith("record ") })
    eventually("recording") { rig.camera.state.value.phase == CameraPhase.RECORDING }
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
    eventually("released") { rig.backend.count("unbind") == 1 && rig.camera.state.value.phase == CameraPhase.IDLE }
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
  fun inv_capture_34_aBindCancelledWhileTheMicrophoneWaitsGivesItsSessionMemoryBack() {
    val rig = rig(moment) { _, counters -> counters.focusAnswer = com.noma4i.medialayer.FocusAnswer.DELAYED }
    rig.attach()
    eventually("the session memory is taken while the microphone waits") { rig.counters.taken.get() == 1 }
    rig.detach()
    eventually("the session memory is given back") { rig.counters.released.get() == rig.counters.taken.get() }
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

    assertThrows(MediaException.InvalidSpec::class.java) { MediaCamera(context, broken) }
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

  private suspend fun settled(rig: Rig, phase: CameraPhase): com.noma4i.medialayer.CameraState =
    withTimeout(5_000) { rig.camera.state.first { it.phase == phase && it.outcome != null } }

  private suspend fun recordingTake(rig: Rig): ULong {
    rig.camera.startRecording()
    return checkNotNull(withTimeout(5_000) { rig.camera.state.first { it.phase == CameraPhase.RECORDING } }.take)
  }

  @Test
  fun inv_capture_77_a_take_ended_by_the_limit_shows_its_captured_outcome_without_a_stop() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    val take = recordingTake(rig)

    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))

    val shown = settled(rig, CameraPhase.READY)
    assertEquals(take to TakeOutcome.Captured(take), shown.take to shown.outcome)
  }

  @Test
  fun inv_capture_36_every_thrown_away_take_shows_its_refusal_and_fault() = runBlocking {
    AppPath().use { app ->
      var call: MediaCallAudio? = null
      val rows = listOf<Triple<CameraRefusal, CameraPhase, (Rig) -> Unit>>(
        Triple(CameraRefusal.IN_CALL, CameraPhase.READY, { _ -> call = app.layer.callAudio(MediaCallKind.Unmanaged) }),
        Triple(CameraRefusal.MICROPHONE_SILENT, CameraPhase.READY, { rig -> rig.backend.audioIssue(CaptureAudioIssue.SILENCED) }),
        Triple(CameraRefusal.TOO_SHORT, CameraPhase.READY, { rig -> rig.counters.info = MediaInfo(400UL, null); rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED)) }),
        Triple(CameraRefusal.CONFIGURE_FAILED, CameraPhase.FAILED, { rig -> rig.backend.emit(CaptureEvent.Fault(FaultKind.OTHER)) })
      )
      val seen = rows.map { (reason, phase, cause) ->
        val rig = rig(moment, app = app)
        ready(rig)
        recordingTake(rig)

        cause(rig)

        val shown = settled(rig, phase)
        call?.close()
        call = null
        rig.camera.close()
        Triple(reason, shown.outcome, shown.fault)
      }
      assertEquals(rows.map { Triple(it.first, TakeOutcome.Refused(it.first), it.first) }, seen)
    }
  }

  @Test
  fun inv_capture_37_a_reset_during_a_take_shows_the_lost_take_and_opens_the_camera_again() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    recordingTake(rig)

    rig.backend.emit(CaptureEvent.Fault(FaultKind.RESET))

    assertEquals(TakeOutcome.Lost(CameraRefusal.INTERRUPTED), settled(rig, CameraPhase.READY).outcome)
    assertEquals(2, rig.backend.count("bind BACK"))
  }

  @Test
  fun inv_capture_38_a_take_shows_recording_only_after_the_microphone_sound_flows() = runBlocking {
    val rig = rig(moment)
    rig.backend.soundOnRecord = false
    ready(rig)
    val phases = CopyOnWriteArrayList<CameraPhase>()
    val watch = launch(Dispatchers.Unconfined) { rig.camera.state.collect { phases.add(it.phase) } }
    val starting = async(Dispatchers.Default) { rig.camera.startRecording() }
    eventually("the hardware began the take") { rig.backend.count("record BACK") == 1 }
    val beforeSound = phases.toList()

    rig.backend.audioIssue(CaptureAudioIssue.ACTIVE)

    withTimeout(5_000) { starting.await() }
    withTimeout(5_000) { rig.camera.state.first { it.phase == CameraPhase.RECORDING } }
    watch.cancel()
    assertFalse("recording was shown before any sound: $beforeSound", beforeSound.contains(CameraPhase.RECORDING))
  }

  @Test
  fun inv_capture_39_closing_the_layer_closes_a_previewed_camera_and_releases_it() = runBlocking {
    val app = AppPath()
    val rig = rig(moment, app = app)
    ready(rig)

    app.close()

    assertEquals(CameraPhase.CLOSED, withTimeout(5_000) { rig.camera.state.first { it.phase == CameraPhase.CLOSED } }.phase)
    eventually("released") { rig.backend.count("unbind") == 1 && rig.counters.released.get() == rig.counters.taken.get() }
  }

  @Test
  fun inv_capture_40_a_flash_or_zoom_the_system_refuses_is_a_failure_event_and_the_state_keeps_the_old_value() = runBlocking {
    AppPath().use { app ->
      val rig = rig(moment, app = app)
      ready(rig)
      val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.take(2).toList() }
      rig.backend.torchFailure = IllegalStateException("torch busy")
      rig.backend.zoomFailure = IllegalArgumentException("zoom out of range")

      rig.camera.setFlash(true)
      eventually("torch tried") { rig.backend.count("torch true") == 1 }
      rig.camera.zoomBy(2.0)

      assertEquals(listOf(
        NativeFailure(FailureSource.CAMERA, "java.lang.IllegalStateException", 0L, "torch busy"),
        NativeFailure(FailureSource.CAMERA, "java.lang.IllegalArgumentException", 0L, "zoom out of range")
      ), withTimeout(5_000) { failures.await() })
      assertEquals(false to 1f, rig.camera.state.value.flash to rig.camera.state.value.zoom)
    }
  }

  @Test
  fun inv_capture_42_a_lens_switch_with_the_flash_on_reaches_ready_and_lights_the_torch_on_the_new_lens() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    rig.camera.setFlash(true)
    eventually("flash on") { rig.camera.state.value.flash }

    rig.camera.setLens(CameraLens.FRONT)

    eventually("front ready") { rig.camera.state.value.lens == CameraLens.FRONT && rig.camera.state.value.phase == CameraPhase.READY }
    assertEquals(true to 2, rig.camera.state.value.flash to rig.backend.count("torch true"))
  }

  @Test
  fun inv_capture_41_a_frame_observer_that_arrives_during_a_take_gets_frames_after_it_without_a_rebind_of_the_recording() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    recordingTake(rig)
    val binds = rig.backend.log.count { it.startsWith("bind") }

    val listening = rig.camera.observeFrames(10.0, ImageAnalysis.Analyzer { it.close() })

    assertNull("the frame output was added to the running recording", rig.backend.tap)
    assertEquals("the running recording was bound again", binds, rig.backend.log.count { it.startsWith("bind") })
    rig.camera.stopRecording()
    eventually("the frame output stands after the take") { rig.backend.tap != null }
    listening.close()
  }

  @Test
  fun inv_audio_66_closing_the_layer_during_a_take_gives_the_focus_back_only_after_the_take_stopped() {
    val app = AppPath()
    val rig = rig(moment, app = app)
    ready(rig)
    val started = CoroutineScope(Dispatchers.Default).async { recordingTake(rig) }
    started.invokeOnCompletion { TestSignal.fire() }
    eventually("the take started") { shadowOf(Looper.getMainLooper()).idle(); started.isCompleted }
    runBlocking { started.await() }
    rig.backend.holdStops = true
    val abandonsBefore = app.focus.calls.count { it == "abandon" }

    val closed = CoroutineScope(Dispatchers.Default).async(start = CoroutineStart.UNDISPATCHED) { app.layer.awaitClosed() }
    closed.invokeOnCompletion { TestSignal.fire() }
    app.close()
    eventually("the take is stopping") { rig.backend.heldStops.isNotEmpty() }

    assertEquals("the focus was given back before the take stopped", abandonsBefore, app.focus.calls.count { it == "abandon" })
    assertFalse("the layer closed before the take stopped", closed.isCompleted)
    app.focus.calls.add("take stopped")
    rig.backend.finalizeHeldStops()

    eventually("the focus was given back") { app.focus.calls.count { it == "abandon" } > abandonsBefore }
    eventually("the layer closed") { shadowOf(Looper.getMainLooper()).idle(); closed.isCompleted }
    runBlocking { closed.await() }
    assertEquals(listOf("take stopped", "abandon"), app.focus.calls.toList().takeLast(2))
  }

  private fun cameraThreads(): Set<Thread> = Thread.getAllStackTraces().keys.filter { it.name == "medialayer-camera" && it.isAlive }.toSet()

  @Test
  fun inv_capture_72_a_cancelled_take_photo_leaves_no_file_nobody_receives() = runBlocking {
    val rig = rig()
    ready(rig)
    val gate = java.util.concurrent.CountDownLatch(1)
    rig.backend.photoGate = gate
    val asked = CoroutineScope(Dispatchers.Default).launch { rig.camera.takePhoto() }
    eventually("the photo is shooting") { rig.camera.state.value.shooting }

    asked.cancelAndJoin()
    gate.countDown()

    eventually("the shot ended") { !rig.camera.state.value.shooting }
    rig.camera.awaitQueue()
    assertEquals(emptyList<String>(), rig.files())
  }

  @Test
  fun inv_capture_73_closing_the_layer_stops_the_camera_thread() {
    val before = cameraThreads()
    val app = AppPath()
    val rig = rig(moment, app = app)
    ready(rig)
    val mine = cameraThreads() - before
    assertTrue(mine.isNotEmpty())

    app.close()

    eventually("the camera thread ended") { mine.none { it.isAlive } }
  }

  @Test
  fun inv_capture_74_a_frame_analyzer_may_fetch_a_clip_without_waiting_on_the_camera_queue() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    val take = recordingTake(rig)
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    settled(rig, CameraPhase.READY)
    val fetched = java.util.concurrent.atomic.AtomicReference<CapturedClip?>()
    val ran = java.util.concurrent.CountDownLatch(1)
    val listening = rig.camera.observeFrames(1000.0, ImageAnalysis.Analyzer { image ->
      fetched.set(rig.camera.clip(take))
      image.close()
      ran.countDown()
    })
    eventually("the frame output stands") { rig.backend.tap != null }
    val image = java.lang.reflect.Proxy.newProxyInstance(javaClass.classLoader, arrayOf(androidx.camera.core.ImageProxy::class.java)) { _, _, _ -> null } as androidx.camera.core.ImageProxy

    rig.backend.tap!!.analyze(image)

    assertTrue("the analyzer waited for the queue it runs on", ran.await(5, java.util.concurrent.TimeUnit.SECONDS))
    assertNotNull(fetched.get())
    listening.close()
  }

  @Test
  fun inv_capture_70_a_clip_stays_for_its_take_after_the_next_take_starts_and_goes_with_the_camera() = runBlocking {
    val rig = rig(moment)
    ready(rig)
    val first = recordingTake(rig)
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    settled(rig, CameraPhase.READY)
    rig.camera.setLens(CameraLens.FRONT)
    eventually("the other side") { rig.camera.state.value.phase == CameraPhase.READY && rig.camera.state.value.lens == CameraLens.FRONT }
    val second = recordingTake(rig)
    assertNotEquals(first, second)

    val clip = checkNotNull(rig.camera.clip(first))

    assertEquals(CameraLens.BACK, clip.lens)
    assertTrue(clip.file.isFile)
    assertNull(rig.camera.clip(first))
    rig.backend.emit(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED))
    settled(rig, CameraPhase.READY)
    rig.camera.close()
    eventually("the clip of the last take went with the camera") { rig.files().isEmpty() || rig.files() == listOf(clip.file.name) }
    assertNull(rig.camera.clip(second))
  }

  @Test
  fun inv_capture_71_a_stop_waiting_when_the_layer_closes_gets_interrupted() = runBlocking {
    val app = AppPath()
    val rig = rig(moment, app = app)
    rig.counters.videoGate = CompletableDeferred()
    ready(rig)
    recordingTake(rig)
    val stopping = CoroutineScope(Dispatchers.Default).async { runCatching { rig.camera.stopRecording() } }
    eventually("the clip is read") { rig.counters.straightened.isNotEmpty() }

    app.close()

    val failure = withTimeout(5_000) { stopping.await() }.exceptionOrNull()
    assertEquals(CameraRefusal.INTERRUPTED, (failure as? MediaException.Camera)?.reason)
  }
}

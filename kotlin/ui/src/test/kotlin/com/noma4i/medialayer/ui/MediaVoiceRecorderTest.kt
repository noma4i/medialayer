package com.noma4i.medialayer.ui

import android.app.Application
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.voiceLevel
import com.noma4i.medialayer.voiceRefusalText
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private class VoiceFakeCapture(private val refusal: VoiceRefusal? = null, private val peaks: List<Float> = listOf(0.5f)) : VoiceCapture {
  val reads = AtomicInteger()
  var limit: (() -> Unit)? = null
  var limitMs = 0L
  private var file: File? = null

  override fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit) {
    refusal?.let { throw VoiceRefused(it) }
    this.file = file
    limit = onLimit
    limitMs = maxDurationMs
  }

  override fun peak(): Float = peaks[reads.getAndIncrement() % peaks.size]

  override fun stop() {
    file!!.writeBytes(byteArrayOf(1, 2, 3))
  }
}

private class VoiceTicks(var now: Long = 1_000) {
  fun read(): Long = now
}

@RunWith(MediaLayerTestRunner::class)
class MediaVoiceRecorderTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()
  private val scope = CoroutineScope(Dispatchers.Default)

  private fun file(): File = File.createTempFile("voice", ".m4a").apply { delete(); deleteOnExit() }

  private fun makeRecorder(capture: VoiceCapture, ticks: VoiceTicks = VoiceTicks(), audio: MediaAudioSession? = null, maxMs: Long = 60_000) =
    MediaVoiceRecorder(capture, maxMs, { audio }, ticks::read, 5, scope)

  @Test
  fun inv_voice_04_aRecordingGivesAnExistingFileAndALengthAboveZero() = runBlocking {
    val ticks = VoiceTicks()
    val capture = VoiceFakeCapture()
    val recorder = makeRecorder(capture, ticks)
    val target = file()

    recorder.start(target)
    ticks.now += 2_500
    val recording = recorder.stop()

    assertEquals(target, recording.file)
    assertTrue(recording.file.isFile)
    assertEquals(2_500L, recording.durationMs)
    assertFalse(recorder.isRecording)
  }

  @Test
  fun inv_voice_05_theLevelsFollowThePeaksThroughTheCoreFormulaAndStayInZeroToOne() = runBlocking {
    val peaks = listOf(0.0f, 0.03f, 1.0f, 5.0f)
    val ticks = VoiceTicks()
    val recorder = makeRecorder(VoiceFakeCapture(peaks = peaks), ticks)
    val seen = CopyOnWriteArrayList<Float>()
    val collecting = scope.launch { recorder.levels.take(4).toList(seen) }
    delay(50)

    recorder.start(file())
    withTimeout(10_000) { collecting.join() }
    ticks.now += 1_000
    recorder.stop()

    assertEquals(peaks.map { voiceLevel(it) }, seen.toList())
    assertTrue(seen.all { it in 0f..1f })
  }

  @Test
  fun inv_voice_06_theLevelIsNotReadInRestOrAfterTheRecordingEnds() = runBlocking {
    val capture = VoiceFakeCapture()
    val ticks = VoiceTicks()
    val recorder = makeRecorder(capture, ticks)
    delay(60)
    assertEquals(0, capture.reads.get())

    recorder.start(file())
    withTimeout(10_000) { while (capture.reads.get() < 2) delay(5) }
    ticks.now += 1_000
    recorder.stop()
    val after = capture.reads.get()
    delay(60)

    assertEquals(after, capture.reads.get())
  }

  @Test
  fun inv_voice_07_aRefusedPermissionIsAMediaExceptionWithTheTextOfTheCore() {
    val denied = makeRecorder(VoiceFakeCapture(refusal = VoiceRefusal.PERMISSION_DENIED))
    val busy = makeRecorder(VoiceFakeCapture(refusal = VoiceRefusal.MICROPHONE_BUSY))
    val target = file()

    val deniedError = assertThrows(MediaException.Voice::class.java) { denied.start(target) }
    val busyError = assertThrows(MediaException.Voice::class.java) { busy.start(target) }

    assertEquals(voiceRefusalText(VoiceRefusal.PERMISSION_DENIED), deniedError.detail)
    assertEquals(voiceRefusalText(VoiceRefusal.MICROPHONE_BUSY), busyError.detail)
    assertFalse(denied.isRecording)
  }

  @Test
  fun inv_voice_07_theSystemCaptureWithoutTheMicrophonePermissionIsRefused() {
    val failure = assertThrows(VoiceRefused::class.java) { SystemVoiceCapture(context).start(file(), 1_000) {} }

    assertEquals(VoiceRefusal.PERMISSION_DENIED, failure.reason)
  }

  @Test
  fun inv_voice_08_theRecordingTakesTheAudioFocusAndGivesItBackAtTheEnd() = runBlocking {
    val port = FakeFocusPort()
    val session = MediaAudioSession(MediaAudioFocus.Mix, port, releaseDelayMs = 0)
    val ticks = VoiceTicks()
    val recorder = makeRecorder(VoiceFakeCapture(), ticks, audio = session)

    recorder.start(file())
    assertTrue(session.isRecording)
    ticks.now += 1_000
    recorder.stop()

    assertFalse(session.isRecording)
  }

  @Test
  fun inv_voice_08_aRefusedAudioFocusIsABusyMicrophoneAndStartsNoCapture() {
    val session = MediaAudioSession(MediaAudioFocus.Mix, FakeFocusPort(grant = false), releaseDelayMs = 0)
    val capture = VoiceFakeCapture()
    val recorder = makeRecorder(capture, audio = session)

    val error = assertThrows(MediaException.Voice::class.java) { recorder.start(file()) }

    assertEquals(voiceRefusalText(VoiceRefusal.MICROPHONE_BUSY), error.detail)
    assertEquals(0L, capture.limitMs)
    assertFalse(session.isRecording)
  }

  @Test
  fun inv_voice_12_theEncoderIsSetTo44100HzOneChannelAnd128000BitPerSecond() {
    val calls = mutableListOf<String>()

    applyVoiceEncoder(object : EncoderTarget {
      override fun setChannels(count: Int) { calls += "channels $count" }
      override fun setSampleRate(hertz: Int) { calls += "rate $hertz" }
      override fun setBitRate(bitsPerSecond: Int) { calls += "bitrate $bitsPerSecond" }
    })

    assertEquals(listOf("channels 1", "rate 44100", "bitrate 128000"), calls)
  }

  @Test
  fun inv_voice_09_theLimitEndsTheRecordingByItselfAndStopGivesItsFile() = runBlocking {
    val ticks = VoiceTicks()
    val capture = VoiceFakeCapture()
    val recorder = makeRecorder(capture, ticks, maxMs = 30_000)

    recorder.start(file())
    assertEquals(30_000L, capture.limitMs)
    ticks.now += 30_000
    capture.limit!!.invoke()
    withTimeout(10_000) { while (recorder.isRecording) delay(5) }
    val recording = recorder.stop()

    assertEquals(30_000L, recording.durationMs)
    assertTrue(recording.file.isFile)
  }

  @Test
  fun inv_voice_10_aRecordingWithoutSoundIsAMediaExceptionAndLeavesNoFile() = runBlocking {
    val capture = object : VoiceCapture {
      override fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit) {}
      override fun peak(): Float = 0f
      override fun stop() {}
    }
    val ticks = VoiceTicks()
    val recorder = makeRecorder(capture, ticks)
    val target = file()

    recorder.start(target)
    ticks.now += 1_000
    val error = runCatching { recorder.stop() }.exceptionOrNull() as MediaException.Voice

    assertEquals(voiceRefusalText(VoiceRefusal.EMPTY), error.detail)
    assertFalse(target.exists())
  }
}

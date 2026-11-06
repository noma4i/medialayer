package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.voiceRefusalText
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private class CountingCapture : VoiceCapture {
  val starts = AtomicInteger()
  private var file: File? = null

  override fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit) {
    starts.incrementAndGet()
    this.file = file
    file.writeBytes(byteArrayOf(1, 2, 3))
  }

  override fun peak(): Float = 0.5f

  override fun stop() {}
}

@RunWith(MediaLayerTestRunner::class)
class CallHoldRecordingTest {
  private val scope = CoroutineScope(Dispatchers.Default)
  private val gate = CallGate()
  private val ticks = java.util.concurrent.atomic.AtomicLong(1_000)

  private fun file(): File = File.createTempFile("call", ".m4a").apply { delete(); deleteOnExit() }

  private fun makeRecorder(capture: VoiceCapture) = MediaVoiceRecorder(capture, 60_000, { null }, { ticks.addAndGet(500) }, 5, scope, gate)

  @Test
  fun inv_call_09_aCallStopsARunningRecordingWithTheInCallRefusalAndDeletesTheFile() = runBlocking {
    val recorder = makeRecorder(CountingCapture())
    val target = file()
    recorder.start(target)
    assertTrue(recorder.isRecording)

    gate.hold()
    withTimeout(10_000) { while (recorder.isRecording) delay(5) }
    val error = runCatching { recorder.stop() }.exceptionOrNull() as MediaException.Voice

    assertEquals(voiceRefusalText(VoiceRefusal.IN_CALL), error.detail)
    assertFalse(target.exists())
  }

  @Test
  fun inv_call_10_aRecordingStartedDuringACallIsRefusedBeforeTheMicrophone() = runBlocking {
    val capture = CountingCapture()
    val recorder = makeRecorder(capture)
    gate.hold()

    val error = runCatching { recorder.start(file()) }.exceptionOrNull() as MediaException.Voice

    assertEquals(voiceRefusalText(VoiceRefusal.IN_CALL), error.detail)
    assertEquals(0, capture.starts.get())
    assertFalse(recorder.isRecording)
  }

  @Test
  fun inv_call_11_aRecordingIsAllowedAfterTheLastHoldIsReleased() = runBlocking {
    val recorder = makeRecorder(CountingCapture())
    gate.hold()
    gate.hold()
    gate.release()
    assertTrue(gate.inCall)
    gate.release()

    recorder.start(file())

    assertTrue(recorder.isRecording)
    assertTrue(recorder.stop().file.isFile)
  }
}

package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioFormat
import android.media.MediaRecorder
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordingKind
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executor
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MicrophoneReleaseOrderTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val fixture = AudioFixture(context)
  private val log = CopyOnWriteArrayList<String>()

  @After fun close() = fixture.close()

  /** An owner whose microphone the system gives back only when the test says so. */
  private inner class Owner(val name: String) : AudioInputOwner {
    @Volatile var released: CompletableDeferred<Unit>? = null
    override fun restore(input: AudioInputHandle, generation: ULong) { log.add("restore $name"); TestSignal.fire() }
    override fun stop(generation: ULong) { log.add("stop $name"); TestSignal.fire() }
    override suspend fun idle() { released?.await() }
  }

  private fun eventually(condition: String, holds: () -> Boolean) = TestSignal.await(condition, 5_000, holds)

  @Test
  fun inv_voice_38_the_voice_input_starts_only_after_the_preview_released_its_audio_record() = runBlocking {
    val link = fixture.link
    val camera = Owner("camera")
    val preview = (link.admit(RecordingKind.VIDEO) as Admission.Admitted).ticket
    val held = link.openInput(RecordingKind.VIDEO, camera)
    eventually("the preview holds the microphone") { log.toList() == listOf("restore camera") }
    val released = CompletableDeferred<Unit>()
    camera.released = released

    val ticket = (link.admit(RecordingKind.VOICE) as Admission.Admitted).ticket
    val voice = Owner("voice")
    val input = link.openInput(RecordingKind.VOICE, voice)
    eventually("the preview was stopped") { "stop camera" in log }
    Thread.sleep(300)
    assertEquals("the voice started while the AudioRecord of the preview was still held", listOf("restore camera", "stop camera"), log.toList())
    released.complete(Unit)
    eventually("the voice started on the release") { log.toList() == listOf("restore camera", "stop camera", "restore voice") }
    input.close(); ticket.close(); held.close(); preview.close()
  }

  @Test
  fun inv_voice_38_a_start_on_a_closed_microphone_leaves_it_idle() = runBlocking {
    val listener = object : RecordingAudioListener {
      override fun onLive(generation: Long) = Unit
      override fun onTimebase(generation: Long, originNanos: Long) = Unit
      override fun onPcm(chunk: PcmChunk) = Unit
      override fun onSilenced(generation: Long, silenced: Boolean) = Unit
      override fun onFailed(generation: Long, failure: NativeFailure) = Unit
    }
    val audio = RecordingAudio(listener, { AutoCloseable {} }, Executor { it.run() })
    audio.close()
    assertThrows(IllegalStateException::class.java) { audio.start(PcmSpec(MediaRecorder.AudioSource.MIC, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)) }
    withTimeout(1_000) { audio.idle() }
    assertTrue(true)
  }
}

package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CallState
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.FocusChange
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.VoiceOutcome
import com.noma4i.medialayer.VoicePhase
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.VoiceState
import java.io.File
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** The microphone side of Android: it writes the file, and reports silence, the limit and its own failure as the real capture does. */
private class VoicePort : VoiceCapture {
  @Volatile var refusal: VoiceRefusal? = null
  @Volatile var stopRefusal: VoiceRefused? = null
  @Volatile var reachLimit: () -> Unit = {}
  @Volatile private var failure: (Exception) -> Unit = {}

  override suspend fun start(file: File, maxDurationMs: ULong, ticket: RecordingTicket, onLimit: () -> Unit, onFailure: (Exception) -> Unit) {
    refusal?.let { throw VoiceRefused(it) }
    reachLimit = onLimit
    failure = onFailure
    file.writeBytes(byteArrayOf(1, 2, 3))
  }
  override fun peak(): Float = 0.5f
  override fun stop() { stopRefusal?.let { throw it } }
  fun silence() = failure(VoiceRefused(VoiceRefusal.MICROPHONE_SILENT))
  fun fail(native: NativeFailure) = failure(VoiceNativeFailure(native))
}

/** Records what the focus port had done at the moment the microphone stops. */
private class StopWitness(private val focus: AppFocusPort) : VoiceCapture {
  @Volatile var callsAtStop: List<String>? = null
  override suspend fun start(file: File, maxDurationMs: ULong, ticket: RecordingTicket, onLimit: () -> Unit, onFailure: (Exception) -> Unit) { file.writeBytes(byteArrayOf(1, 2, 3)) }
  override fun peak(): Float = 0.5f
  override fun stop() { callsAtStop = focus.calls.toList(); TestSignal.fire() }
}

@RunWith(MediaLayerTestRunner::class)
class VoiceContractTest {
  private val app = AppPath()
  private val port = VoicePort()
  @Volatile private var now = 1_000uL
  private val recorder = MediaVoiceRecorder(capture = port, maxDurationMs = 60_000u, clock = { now })

  @After fun close() = app.close()

  private fun file(): File = File.createTempFile("voice", ".m4a").apply { delete(); deleteOnExit() }

  private suspend fun take(file: File): ULong {
    recorder.start(file)
    return withTimeout(5_000) { recorder.state.first { it.phase == VoicePhase.RECORDING } }.take
  }

  private suspend fun outcome(take: ULong): VoiceState = withTimeout(5_000) { recorder.state.first { it.take == take && it.outcome != null } }

  private suspend fun available() = withTimeout(5_000) { app.layer.microphoneAvailability.first { it == MicrophoneAvailability.AVAILABLE } }

  @Test
  fun inv_voice_13_an_accepted_start_shows_a_new_recording_take() = runBlocking {
    assertEquals(VoiceState(0u, VoicePhase.IDLE, null, 0uL), recorder.state.value)

    recorder.start(file())

    assertEquals(VoiceState(1u, VoicePhase.RECORDING, null, 0uL), withTimeout(5_000) { recorder.state.first { it.phase == VoicePhase.RECORDING } }.copy(recordedMs = 0uL))
  }

  @Test
  fun inv_audio_41_every_revocation_ends_a_voice_take_with_its_refusal_and_no_file() = runBlocking {
    var call: MediaCallAudio? = null
    val rows = listOf<Triple<VoiceRefusal, () -> Unit, suspend () -> Unit>>(
      Triple(VoiceRefusal.INTERRUPTED, { app.focus.send(FocusSignal.Change(FocusChange.LOSS_TRANSIENT)) }, { app.focus.send(FocusSignal.Change(FocusChange.GAIN)) }),
      Triple(VoiceRefusal.IN_CALL, { call = app.layer.callAudio(MediaCallKind.Unmanaged) }, { call!!.close(); withTimeout(5_000) { call!!.states.first { it == CallState.ENDED } } }),
      Triple(VoiceRefusal.MICROPHONE_SILENT, { port.silence() }, {}),
      Triple(VoiceRefusal.PERMISSION_DENIED, { app.permission.change(false) }, { app.permission.change(true); available() }),
      Triple(VoiceRefusal.INTERRUPTED, { app.close() }, {})
    )
    val seen = rows.map { (reason, cause, undo) ->
      val target = file()
      val take = take(target)

      cause()

      val ended = outcome(take)
      undo()
      Triple(ended, target.exists(), reason)
    }
    seen.forEach { (ended, exists, reason) ->
      assertEquals("the app sees the end of the take and its refusal without a stop", VoiceState(ended.take, VoicePhase.IDLE, VoiceOutcome.Refused(reason), 0uL), ended)
      assertFalse("$reason left its file", exists)
    }
  }

  @Test
  fun inv_voice_14_the_duration_limit_shows_the_recorded_file_without_a_stop() = runBlocking {
    val target = file()
    val take = take(target)
    now = 3_000uL

    port.reachLimit()

    assertEquals(VoiceState(take, VoicePhase.IDLE, VoiceOutcome.Recorded(target.absolutePath, 2_000uL), 0uL), outcome(take))
    assertTrue(target.isFile)
  }

  @Test
  fun inv_voice_15_a_recorder_failure_shows_the_native_failure_deletes_the_file_and_frees_the_microphone() = runBlocking {
    val target = file()
    val take = take(target)
    val failure = NativeFailure(FailureSource.RECORDER, "android.media.MediaRecorder", 100L, "media server died")

    port.fail(failure)

    assertEquals(VoiceState(take, VoicePhase.IDLE, VoiceOutcome.Failed(failure), 0uL), outcome(take))
    assertFalse(target.exists())
    assertEquals(take + 1u, take(file()))
  }

  @Test
  fun inv_voice_16_every_refusal_carries_its_reason_in_the_thrown_error() = runBlocking {
    suspend fun refusal(work: suspend () -> Unit): VoiceRefusal = assertThrows(MediaException.Voice::class.java) { runBlocking { work() } }.reason
    val seen = linkedMapOf<String, VoiceRefusal>()

    app.permission.change(false)
    seen["permission"] = refusal { recorder.start(file()) }
    app.permission.change(true)
    available()

    port.refusal = VoiceRefusal.MICROPHONE_BUSY
    seen["busy"] = refusal { recorder.start(file()) }
    port.refusal = null
    assertEquals("a refused start shows its refusal in the state", VoiceOutcome.Refused(VoiceRefusal.MICROPHONE_BUSY), recorder.state.value.outcome)

    val call = app.layer.callAudio(MediaCallKind.Unmanaged)
    withTimeout(5_000) { call.states.first { it == CallState.LIVE } }
    seen["call"] = refusal { recorder.start(file()) }
    call.close()
    withTimeout(5_000) { call.states.first { it == CallState.ENDED } }

    take(file())
    seen["empty"] = refusal { recorder.stop() }

    val interrupted = take(file())
    app.focus.send(FocusSignal.Change(FocusChange.LOSS_TRANSIENT))
    outcome(interrupted)
    seen["revoked"] = refusal { recorder.stop() }

    assertEquals(mapOf(
      "permission" to VoiceRefusal.PERMISSION_DENIED, "busy" to VoiceRefusal.MICROPHONE_BUSY, "call" to VoiceRefusal.IN_CALL,
      "empty" to VoiceRefusal.EMPTY, "revoked" to VoiceRefusal.INTERRUPTED
    ), seen)
  }

  @Test
  fun inv_voice_17_the_levels_of_a_take_complete_when_the_take_ends() = runBlocking {
    take(file())
    val stopped = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { recorder.levels.toList() }
    now = 2_000uL
    recorder.stop()
    withTimeout(5_000) { stopped.await() }

    take(file())
    val revoked = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { recorder.levels.toList() }
    app.focus.send(FocusSignal.Change(FocusChange.LOSS_TRANSIENT))
    withTimeout(5_000) { revoked.await() }
    Unit
  }

  @Test
  fun inv_voice_18_a_call_takes_the_communication_mode_only_after_the_recorder_hardware_stopped() = runBlocking {
    val witness = StopWitness(app.focus)
    val witnessed = MediaVoiceRecorder(capture = witness, maxDurationMs = 60_000u, clock = { now })
    witnessed.start(file())
    withTimeout(5_000) { witnessed.state.first { it.phase == VoicePhase.RECORDING } }

    val call = app.layer.callAudio(MediaCallKind.Unmanaged)

    TestSignal.await("the call took the communication mode", 5_000) { app.focus.calls.any { it.startsWith("communication speaker") } }
    val atStop = witness.callsAtStop
    assertTrue("the revoked take never stopped the hardware", atStop != null)
    assertFalse("the call took the communication mode before the recorder stopped: $atStop", atStop!!.any { it.startsWith("communication speaker") })
    call.close()
  }

  @Test
  fun inv_voice_19_a_take_file_that_cannot_be_removed_is_one_file_failure_event() = runBlocking {
    val failure = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.first { it.source == FailureSource.FILE } }
    val folder = java.nio.file.Files.createTempDirectory("voice").toFile()
    val take = take(File(folder, "take.m4a"))
    folder.setWritable(false)
    try {
      app.focus.send(FocusSignal.Change(FocusChange.LOSS_TRANSIENT))

      assertEquals(VoiceState(take, VoicePhase.IDLE, VoiceOutcome.Refused(VoiceRefusal.INTERRUPTED), 0uL), outcome(take))
      assertEquals(FailureSource.FILE, withTimeout(5_000) { failure.await() }.source)
    } finally {
      folder.setWritable(true)
      folder.deleteRecursively()
    }
  }
}

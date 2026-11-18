package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PlayRefusal
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.flow.first
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.CallState
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.FocusAnswer
import com.noma4i.medialayer.FocusChange
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.RecordingRefusal
import com.noma4i.medialayer.SessionWant
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.voiceRefusalText
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

internal class RecordingFocusPort : AudioFocusPort {
  val calls = SignallingList<String>()
  var answer = FocusAnswer.GRANTED
  @Volatile var onRequest: (FocusPlan) -> Unit = {}
  private var sink: (FocusSignal) -> Unit = {}

  override fun listen(sink: (FocusSignal) -> Unit) { this.sink = sink }
  fun send(signal: FocusSignal) = sink(signal)

  override fun request(plan: FocusPlan): FocusAnswer {
    calls.add("request ${plan.gain} ${plan.usage}")
    onRequest(plan)
    return answer
  }

  override fun abandon() { calls.add("abandon") }

  override fun enterCommunication(speaker: Boolean) { calls.add("communication on=true speaker=$speaker") }

  override fun leaveCommunication() { calls.add("communication on=false speaker=false") }
}

internal object GrantedPermission : MicrophonePermissionPort {
  override fun state() = com.noma4i.medialayer.MicrophonePermission.GRANTED
}

@RunWith(MediaLayerTestRunner::class)
class AudioLinkTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val focus = RecordingFocusPort()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("audio-link").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "audio-link"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val permission = TogglePermission()
  private val link = AudioLink(core, AudioPorts(focus, permission))
  private val seen = CopyOnWriteArrayList<MicrophoneAvailability>()
  private val follow = link.onAvailability { seen.add(it); TestSignal.fire() }

  @After fun close() {
    follow.close()
    link.close()
    core.close()
  }

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  @Test
  fun inv_audio_74_an_admission_reads_the_permission_of_that_moment_before_the_capture() = runBlocking {
    permission.changeSilently(false)
    val refused = link.admit(RecordingKind.VOICE)
    assertEquals(Admission.Refused(RecordingRefusal.PERMISSION_DENIED), refused)
    permission.changeSilently(true)
    val admitted = link.admit(RecordingKind.VOICE)
    assertTrue("a grant the callback has not delivered yet admits: $admitted", admitted is Admission.Admitted)
    (admitted as Admission.Admitted).ticket.close()
  }

  @Test
  fun inv_audio_78_a_permission_never_asked_is_not_a_denied_one() = runBlocking {
    permission.change(com.noma4i.medialayer.MicrophonePermission.NOT_ASKED)
    assertEquals(Admission.Refused(RecordingRefusal.PERMISSION_DENIED), link.admit(RecordingKind.VOICE))
    await("never asked is shown") { seen.lastOrNull() == MicrophoneAvailability.PERMISSION_NOT_ASKED }
    permission.change(false)
    await("denied is another value") { seen.lastOrNull() == MicrophoneAvailability.PERMISSION_DENIED }
    assertTrue(seen.toList().containsAll(listOf(MicrophoneAvailability.PERMISSION_NOT_ASKED, MicrophoneAvailability.PERMISSION_DENIED)))
  }

  private fun ownSystemLink(): Triple<MediaLayer, AudioLink, AudioSystem> {
    val system = AudioSystem(context)
    val layer = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("audio-own").toString(), fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "audio-own"), slots = null,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(), audioFocus = AudioFocus.EXCLUSIVE
    ))
    return Triple(layer, AudioLink(layer, AudioPorts(system, permission)), system)
  }

  @Test
  fun inv_audio_11_a_camera_and_a_voice_recorder_hold_the_microphone_together() = runBlocking {
    val (layer, own, system) = ownSystemLink()
    val states = CopyOnWriteArrayList<MicrophoneAvailability>()
    val watch = own.onAvailability { states.add(it); TestSignal.fire() }
    val voice = own.admit(RecordingKind.VOICE) as Admission.Admitted
    val video = own.admit(RecordingKind.VIDEO) as Admission.Admitted
    val revoked = CopyOnWriteArrayList<RecordingRefusal>()
    val hearVoice = voice.ticket.onRevoked { reason, done -> revoked.add(reason); done(); TestSignal.fire() }
    val hearVideo = video.ticket.onRevoked { reason, done -> revoked.add(reason); done(); TestSignal.fire() }

    system.onRecordingConfigs(2)

    system.onRecordingConfigs(3)
    TestSignal.await("a third record is another app", 5_000) { states.lastOrNull() == MicrophoneAvailability.OTHER_APP }

    system.onRecordingConfigs(2)
    TestSignal.await("back to ours", 5_000) { states.lastOrNull() == MicrophoneAvailability.AVAILABLE && states.size == 3 }
    assertEquals("two own records never read as another app", listOf(MicrophoneAvailability.AVAILABLE, MicrophoneAvailability.OTHER_APP, MicrophoneAvailability.AVAILABLE), states.toList())
    assertTrue("no recording is revoked as another app: $revoked", revoked.isEmpty())
    hearVoice.close(); hearVideo.close(); voice.ticket.close(); video.ticket.close(); watch.close(); own.close(); layer.close()
  }

  @Test
  fun inv_audio_34_every_wish_of_the_core_has_one_focus_plan() {
    val table = listOf(
      SessionWant.Playback(AudioFocus.EXCLUSIVE) to FocusPlan.of(SessionWant.Playback(AudioFocus.EXCLUSIVE)),
    )
    assertEquals(FocusPlan(FocusGain.GAIN, FocusUsage.MEDIA, communication = false), table[0].second)
    assertEquals(FocusPlan(FocusGain.GAIN, FocusUsage.MEDIA, communication = false), FocusPlan.of(SessionWant.Playback(AudioFocus.DUCK)))
    assertEquals(null, FocusPlan.of(SessionWant.Playback(AudioFocus.MIX)))
    assertEquals(FocusPlan(FocusGain.GAIN_TRANSIENT_EXCLUSIVE, FocusUsage.MEDIA, communication = false), FocusPlan.of(SessionWant.Recording(RecordingKind.VOICE)))
    assertEquals(FocusPlan(FocusGain.GAIN_TRANSIENT_EXCLUSIVE, FocusUsage.MEDIA, communication = false), FocusPlan.of(SessionWant.Recording(RecordingKind.VIDEO)))
    assertEquals(FocusPlan(FocusGain.GAIN_TRANSIENT, FocusUsage.VOICE_COMMUNICATION, communication = true), FocusPlan.of(SessionWant.Call(com.noma4i.medialayer.CallKind.Unmanaged, false)))
  }

  @Test
  fun inv_audio_36_system_signals_reach_the_core_and_its_actions_reach_the_focus() {
    link.audible(0u, true)
    link.audible(1u, true)
    await("one request for two sounding slots") { focus.calls.toList() == listOf("request GAIN MEDIA") }

    focus.send(FocusSignal.Change(FocusChange.LOSS))
    await("abandoned after the loss") { focus.calls.last() == "abandon" }
    link.audible(0u, false)
    link.audible(1u, false)
    focus.send(FocusSignal.Change(FocusChange.GAIN))
    assertEquals(listOf("request GAIN MEDIA", "abandon"), focus.calls.toList())

    focus.send(FocusSignal.PhoneCall(true))
    await("the core heard the phone call") { seen.lastOrNull() == MicrophoneAvailability.EXTERNAL_CALL }
    focus.send(FocusSignal.PhoneCall(false))
    await("back to the permanently lost session, which needs a new intent") { seen.lastOrNull() == MicrophoneAvailability.OTHER_APP }
    assertFalse("a noisy route never writes the user flag", Media.muted)
  }

  @Test
  fun inv_audio_37_a_focus_refused_in_the_background_is_requested_again_on_the_foreground() {
    focus.answer = FocusAnswer.FAILED
    link.foreground(false)
    link.audible(0u, true)
    await("refused") { focus.calls.toList() == listOf("request GAIN MEDIA") }
    link.audible(0u, false)
    focus.answer = FocusAnswer.GRANTED

    link.foreground(true)
    link.audible(0u, true)

    await("requested again") { focus.calls.count { it == "request GAIN MEDIA" } == 2 }
    assertEquals(listOf("request GAIN MEDIA", "request GAIN MEDIA"), focus.calls.toList())
  }

  @Test
  fun inv_audio_38_a_telecom_call_leaves_focus_and_mode_to_telecom_and_its_object_ends_the_call() {
    val states = CopyOnWriteArrayList<CallState>()
    var call: MediaCallAudio? = link.call(MediaCallKind.Telecom(object : CallEndpointPort { override fun select(speaker: Boolean, done: (CallEndpointResult) -> Unit) = done(CallEndpointResult.Applied) }))
    val watch = call!!.onState { states.add(it); TestSignal.fire() }
    call.systemActivated()
    await("live") { states.toList() == listOf(CallState.CONNECTING, CallState.LIVE) }
    assertEquals("Telecom owns focus, mode and route", emptyList<String>(), focus.calls.toList())
    val player = runBlocking { core.playUrl(clip) }
    player.play()
    assertEquals("a play during the call is refused", PlayRefusal.IN_CALL, runBlocking { withTimeout(5_000) { player.state.first { it.refused != null } }.refused })

    call.close()
    call = null

    await("ended") { states.lastOrNull() == CallState.ENDED }
    watch.close()
    player.play()
    runBlocking { withTimeout(5_000) { player.state.first { it.playing || it.wish && it.refused == null } } }
    player.close()
  }

  private val clip = MediaItem(Kind.VIDEO, "https://o.test/v.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

  @Test
  fun inv_audio_39_an_unmanaged_call_sets_the_communication_mode_and_clears_it_at_the_end() {
    val call = link.call(MediaCallKind.Unmanaged)
    call.setSpeaker(true)
    await("communication with the speaker") { focus.calls.contains("communication on=true speaker=true") }
    focus.send(FocusSignal.CommunicationMode(true))

    call.close()

    await("cleared") { focus.calls.contains("communication on=false speaker=false") }
    val calls = focus.calls.toList()
    assertTrue("the release abandons the focus, then clears the mode: $calls", calls.lastIndexOf("abandon") in 0 until calls.lastIndexOf("communication on=false speaker=false"))
    assertFalse("the own mode is not an external call", seen.contains(MicrophoneAvailability.EXTERNAL_CALL))
  }

  @Test
  fun inv_audio_40_a_silenced_capture_revokes_the_recording_with_its_reason() = runBlocking {
    val admitted = link.admit(RecordingKind.VOICE) as Admission.Admitted
    val revoked = CopyOnWriteArrayList<RecordingRefusal>()
    val watch = admitted.ticket.onRevoked { reason, done -> revoked.add(reason); done(); TestSignal.fire() }

    focus.send(FocusSignal.Silenced(admitted.ticket.id))

    await("revoked") { revoked.toList() == listOf(RecordingRefusal.MICROPHONE_SILENT) }
    watch.close()
    admitted.ticket.close()
    val again = link.admit(RecordingKind.VOICE)
    assertTrue("the microphone is free again: $again", again is Admission.Admitted)
    (again as Admission.Admitted).ticket.close()
  }

  @Test
  fun inv_audio_42_a_recording_without_an_open_layer_is_refused_and_records_nothing() = runBlocking {
    val capture = WritingVoiceCapture()
    val recorder = MediaVoiceRecorder(capture = capture, maxDurationMs = 60_000u, link = { null }, clock = { 1000uL })
    val target = File.createTempFile("voice", ".m4a").also { it.delete() }
    assertThrows(MediaException::class.java) { runBlocking { recorder.start(target) } }
    assertEquals(0, capture.starts)
    assertFalse(target.exists())
  }
  @Test
  fun inv_audio_51_a_call_that_arrives_with_the_admission_leaves_the_microphone_untouched() = runBlocking {
    val capture = WritingVoiceCapture()
    val recorder = MediaVoiceRecorder(capture = capture, maxDurationMs = 60_000u, link = { link }, clock = { 1000uL })
    repeat(20) { round ->
      focus.onRequest = { plan -> if (plan.gain == FocusGain.GAIN_TRANSIENT_EXCLUSIVE) focus.send(FocusSignal.PhoneCall(true)) }
      val target = File.createTempFile("voice", ".m4a").also { it.delete() }
      val error = assertThrows(MediaException.Voice::class.java) { runBlocking { recorder.start(target) } }
      assertEquals("round $round", voiceRefusalText(VoiceRefusal.IN_CALL), error.detail)
      assertEquals("round $round: the microphone was started during the call", 0, capture.starts)
      assertFalse(target.exists())
      focus.onRequest = {}
      focus.send(FocusSignal.PhoneCall(false))
      await("free after round $round") { seen.lastOrNull() == MicrophoneAvailability.AVAILABLE }
    }
  }

  @Test
  fun inv_audio_52_a_new_plan_releases_what_the_previous_plan_held() {
    val mixCore = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("audio-link-mix").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "audio-link-mix"), slots = null,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
      audioFocus = AudioFocus.MIX
    ))
    val mixFocus = RecordingFocusPort()
    val mixLink = AudioLink(mixCore, AudioPorts(mixFocus, GrantedPermission))
    try {
      mixLink.audible(0u, true)
      val call = mixLink.call(MediaCallKind.Unmanaged)
      TestSignal.await("call focus", 5_000) { mixFocus.calls.contains("communication on=true speaker=false") }

      call.close()

      TestSignal.await("released", 5_000) { mixFocus.calls.toList().takeLast(2).toSet() == setOf("communication on=false speaker=false", "abandon") }
    } finally {
      mixLink.close()
      mixCore.close()
    }
  }

  @Test
  fun inv_audio_58_a_refused_call_focus_holds_no_communication_mode() {
    focus.answer = FocusAnswer.FAILED
    val states = CopyOnWriteArrayList<CallState>()
    val call = link.call(MediaCallKind.Unmanaged)
    call.onState { states.add(it); TestSignal.fire() }
    await("asked") { focus.calls.any { it.startsWith("request") } }

    call.close()

    await("ended") { states.lastOrNull() == CallState.ENDED }
    assertFalse("the earpiece mode was set without focus: ${focus.calls}", focus.calls.any { it.startsWith("communication on=true") })
  }

}

internal class WritingVoiceCapture : VoiceCapture {
  @Volatile var starts = 0
  override suspend fun start(file: File, maxDurationMs: ULong, ticket: RecordingTicket, onLimit: () -> Unit, onFailure: (Exception) -> Unit) {
    starts += 1
    file.writeBytes(byteArrayOf(1, 2, 3))
  }
  override fun peak(): Float = 0.5f
  override fun stop() {}
}

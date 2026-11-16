package com.noma4i.medialayer.ui

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.TimeUnit
import java.util.concurrent.CountDownLatch
import com.noma4i.medialayer.NoHandle
import com.noma4i.medialayer.Subscription
import com.noma4i.medialayer.PlaybackListener
import com.noma4i.medialayer.MediaPlayer
import androidx.compose.foundation.layout.Column
import android.os.Handler
import kotlinx.coroutines.flow.toList
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.CallState
import com.noma4i.medialayer.FocusChange
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PauseReason
import com.noma4i.medialayer.PlayRefusal
import com.noma4i.medialayer.PlaybackState
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class ContractSource(override val url: String, override val kind: Kind = Kind.AUDIO) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

/** A system event that pauses playback and the step that takes it back. */
private class Cause(val name: String, val start: () -> Unit, val end: () -> Unit = {})

/** A player of the core whose playback subscription waits on the input thread until the test opens the gate. */
private class HeldPlayer(private val gate: CountDownLatch, private val first: PlaybackState) : MediaPlayer(NoHandle) {
  @Volatile var subscribed = false
  override fun subscribePlayback(listener: PlaybackListener): Subscription {
    gate.await(5, TimeUnit.SECONDS)
    subscribed = true
    listener.changed(first)
    return Subscription(NoHandle)
  }
}

@RunWith(MediaLayerTestRunner::class)
class PlaybackContractTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val app = AppPath()
  private val looper = shadowOf(Looper.getMainLooper())
  private val controls = CopyOnWriteArrayList<AudioControl>()
  private var url by mutableStateOf("file:///contract-0.mp3")
  private var muted by mutableStateOf(false)
  private var rate by mutableStateOf(1f)
  private var group by mutableStateOf<String?>("chat")
  private var suspended by mutableStateOf(false)

  @After fun close() = app.close()

  private fun until(what: String, condition: () -> Boolean) {
    try {
      rule.waitUntil(10_000) { looper.idle(); condition() }
    } catch (error: ComposeTimeoutException) {
      reportComposeTimeout(error, "$what url=$url playing=${controls.lastOrNull()?.isPlaying} play=${app.slots.count("play")} pause=${app.slots.count("pause")}")
    }
  }

  private fun mount(): AudioControl {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(app.layer)) {
        SuspendMediaPlayback(active = suspended)
        MediaAudio(ContractSource(url), muted = muted, rate = rate, group = group) { control ->
          SideEffect { if (controls.lastOrNull() !== control) controls.add(control) }
          Box(Modifier.size(80.dp))
        }
      }
    }
    until("mounted") { controls.isNotEmpty() }
    return controls.last()
  }

  private fun remount(next: String): AudioControl {
    val before = controls.last()
    rule.runOnIdle { url = next }
    until("remounted on $next") { controls.last() !== before }
    return controls.last()
  }

  private fun playing(control: AudioControl) {
    rule.runOnIdle { control.play() }
    until("playing $url") { control.isPlaying && control.isSounding && app.slots.playing(url) }
  }

  private fun paused(control: AudioControl, why: String) =
    until("paused by $why") { !control.isPlaying && !control.isSounding && !app.slots.playing(url) }

  // Every earlier command of the place reached the native player once a later rate reaches it.
  private fun barrier(change: () -> Unit = {}) {
    rule.waitForIdle()
    val before = app.slots.count("rate")
    rule.runOnIdle { change(); rate += 1f }
    until("rate barrier") { app.slots.count("rate") > before }
  }

  private fun ownCall(): Cause {
    var call: MediaCallAudio? = null
    return Cause("own call", { call = app.layer.callAudio(MediaCallKind.Unmanaged) }, {
      call!!.close()
      runBlocking { withTimeout(5_000) { call!!.states.first { it == CallState.ENDED } } }
    })
  }

  private fun voiceTake(): Cause {
    val recorder = MediaVoiceRecorder(capture = WritingVoiceCapture(), maxDurationMs = 60_000u)
    return Cause("voice recording", { runBlocking { recorder.start(File.createTempFile("voice", ".m4a").also { it.delete() }) } }, { runBlocking { runCatching { recorder.stop() } } })
  }

  private fun focus(change: FocusChange) = app.focus.send(FocusSignal.Change(change))

  @Test
  fun inv_playback_01_a_user_play_and_pause_show_the_core_truth() {
    val control = mount()
    playing(control)

    rule.runOnIdle { control.pause() }

    paused(control, "the user")
  }

  @Test
  fun inv_playback_02_every_pause_the_library_makes_shows_paused() {
    mount()
    val causes = listOf(
      Cause("transient focus loss", { focus(FocusChange.LOSS_TRANSIENT) }, { focus(FocusChange.GAIN) }),
      Cause("permanent focus loss", { focus(FocusChange.LOSS) }),
      Cause("private output lost", { app.focus.send(FocusSignal.Noisy) }),
      ownCall(),
      Cause("external call", { app.focus.send(FocusSignal.PhoneCall(true)) }, { app.focus.send(FocusSignal.PhoneCall(false)) }),
      voiceTake(),
      Cause("suspension", { rule.runOnIdle { suspended = true } }, { rule.runOnIdle { suspended = false } })
    )
    causes.forEachIndexed { index, cause ->
      val control = remount("file:///pause-$index.mp3")
      playing(control)

      cause.start()

      paused(control, cause.name)
      cause.end()
    }
  }

  @Test
  fun inv_playback_03_a_resume_with_permission_plays_the_same_place_again() {
    val control = mount()
    playing(control)
    focus(FocusChange.LOSS_TRANSIENT)
    paused(control, "a transient loss")

    focus(FocusChange.GAIN)

    until("resumed") { control.isPlaying && control.isSounding && app.slots.playing(url) }
  }

  @Test
  fun inv_playback_04_after_a_forgotten_wish_one_play_plays() {
    mount()
    val causes = listOf(
      Cause("permanent focus loss", { focus(FocusChange.LOSS) }),
      Cause("private output lost", { app.focus.send(FocusSignal.Noisy) }),
      ownCall()
    )
    causes.forEachIndexed { index, cause ->
      val control = remount("file:///forget-$index.mp3")
      playing(control)
      cause.start()
      paused(control, cause.name)
      cause.end()

      rule.runOnIdle { control.play() }

      until("played again after ${cause.name}") { control.isPlaying && control.isSounding && app.slots.playing(url) }
    }
  }

  @Test
  fun inv_playback_05_a_play_during_a_call_is_refused() {
    val control = mount()
    val player = runBlocking { app.layer.playUrl(MediaItem(Kind.AUDIO, "file:///direct.mp3", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)) }
    val call = app.layer.callAudio(MediaCallKind.Unmanaged)
    runBlocking { withTimeout(5_000) { call.states.first { it == CallState.LIVE } } }

    rule.runOnIdle { control.play() }
    player.play()

    barrier()
    rule.runOnIdle { assertFalse("the place plays during a call", control.isPlaying || control.isSounding) }
    assertFalse(app.slots.playing(url))
    val refused = runBlocking { withTimeout(5_000) { player.state.first { it.refused != null } } }
    assertEquals(false to PlayRefusal.IN_CALL, refused.playing to refused.refused)
    call.close()
    player.close()
  }

  @Test
  fun inv_playback_06_input_changes_after_a_forgotten_wish_play_nothing() {
    val control = mount()
    playing(control)
    focus(FocusChange.LOSS)
    paused(control, "a permanent loss")
    val plays = app.slots.count("play")

    barrier { muted = true; group = "other" }

    rule.runOnIdle { assertFalse(control.isPlaying || control.isSounding) }
    assertEquals(plays, app.slots.count("play"))
    assertFalse(app.slots.playing(url))
  }

  @Test
  fun inv_playback_07_a_video_reports_each_change_of_the_core_playing_truth() {
    val changes = SignallingList<PlaybackState>()
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(app.layer)) {
        MediaVideo(ContractSource("file:///video.mp4", Kind.VIDEO), playing = true, modifier = Modifier.size(80.dp), onPlaybackChange = { changes.add(it) })
      }
    }
    until("playing") { changes.lastOrNull()?.playing == true }

    focus(FocusChange.LOSS_TRANSIENT)
    until("paused") { changes.last().playing == false }
    focus(FocusChange.GAIN)
    until("resumed") { changes.last().playing }

    val playingTruth = changes.map { it.playing }.dropWhile { !it }.fold(mutableListOf<Boolean>()) { shown, value -> if (shown.lastOrNull() != value) shown.add(value); shown }
    assertEquals(listOf(true, false, true), playingTruth)
  }

  @Test
  fun inv_playback_08_play_url_gives_a_library_player_that_a_call_pauses_and_forgets() = runBlocking {
    val player = app.layer.playUrl(MediaItem(Kind.AUDIO, "file:///direct.mp3", emptyMap(), null, null, strategy = CacheStrategy.WINDOW))
    player.play()
    withTimeout(5_000) { player.state.first { it.playing } }

    val call = app.layer.callAudio(MediaCallKind.Unmanaged)

    assertEquals(PlaybackState(playing = false, wish = false, reason = PauseReason.IN_CALL, refused = null, attempt = 0uL), withTimeout(5_000) { player.state.first { !it.playing } })
    call.close()
    withTimeout(5_000) { call.states.first { it == CallState.ENDED } }
    player.play()
    assertEquals(PlaybackState(playing = true, wish = true, reason = null, refused = null, attempt = 0uL), withTimeout(5_000) { player.state.first { it.playing } })
    player.close()
    withTimeout(5_000) { player.states.toList() }
    Unit
  }

  @Test
  fun inv_playback_09_reading_a_player_state_on_main_does_not_wait_for_the_core_subscription() {
    val gate = CountDownLatch(1)
    val player = HeldPlayer(gate, PlaybackState(true, true, null, null, 0uL))
    val nextMainTask = AtomicBoolean()
    try {
      Handler(Looper.getMainLooper()).post { nextMainTask.set(true) }

      val shown = player.state.value
      looper.idle()

      assertFalse("the read on main waited for the core subscription", player.subscribed)
      assertTrue("the next main task did not run while the subscription waits", nextMainTask.get())
      assertEquals(PlaybackState(false, false, null, null, 0uL), shown)
    } finally {
      gate.countDown()
    }
    assertEquals(PlaybackState(true, true, null, null, 0uL), runBlocking { withTimeout(5_000) { player.state.first { it.playing } } })
  }

  @Test
  fun inv_playback_10_audio_and_video_places_show_the_pause_reason_and_the_refusal_of_the_core() {
    val videoStates = SignallingList<PlaybackState>()
    var videoPlaying by mutableStateOf(true)
    var mounted: AudioControl? = null
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(app.layer)) {
        Column {
          MediaAudio(ContractSource(url)) { control ->
            SideEffect { mounted = control }
            Box(Modifier.size(80.dp))
          }
          MediaVideo(ContractSource("file:///video.mp4", Kind.VIDEO), playing = videoPlaying, modifier = Modifier.size(80.dp), onPlaybackChange = { videoStates.add(it) })
        }
      }
    }
    until("mounted") { mounted != null }
    val audio = mounted!!
    rule.runOnIdle { audio.play() }
    until("both wish to play") { audio.playback.wish && videoStates.lastOrNull()?.wish == true }

    val call = app.layer.callAudio(MediaCallKind.Unmanaged)
    val paused = PlaybackState(playing = false, wish = false, reason = PauseReason.IN_CALL, refused = null, attempt = 0uL)
    until("both paused by the call") { audio.playback == paused && videoStates.lastOrNull() == paused }

    rule.runOnIdle { audio.play() }
    videoPlaying = false
    rule.waitForIdle()
    videoPlaying = true
    until("both refused during the call") { audio.playback.refused == PlayRefusal.IN_CALL && videoStates.lastOrNull()?.refused == PlayRefusal.IN_CALL }

    rule.runOnIdle { assertEquals(false to PlayRefusal.IN_CALL, audio.playback.playing to audio.playback.refused) }
    assertEquals(false to PlayRefusal.IN_CALL, videoStates.last().playing to videoStates.last().refused)
    call.close()
  }
}

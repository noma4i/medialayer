package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioManager
import android.os.ParcelUuid
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallEndpointCompat
import androidx.test.core.app.ApplicationProvider
import java.util.UUID
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import org.robolectric.Shadows.shadowOf
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.CallRoute
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.CallState
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.FocusAnswer
import com.noma4i.medialayer.FocusChange
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith

/** The Telecom call endpoint of Android: it answers each speaker request as CallControl does. */
private class TelecomEndpoints : CallEndpointPort {
  val asked = SignallingList<Boolean>()
  @Volatile var accepts = true
  override fun select(speaker: Boolean, done: (CallEndpointResult) -> Unit) {
    asked.add(speaker)
    done(if (accepts) CallEndpointResult.Applied else CallEndpointResult.Failed(NativeFailure(FailureSource.SPEAKER, "androidx.core.telecom", 2L, "Endpoint change refused")))
  }
}

/** A Telecom call of androidx.core.telecom: only its endpoint list and its endpoint change are answered, as the system answers them. */
private class TelecomCall(endpoints: () -> Flow<List<CallEndpointCompat>>, change: () -> CallControlResult) {
  val scope: CallControlScope = java.lang.reflect.Proxy.newProxyInstance(
    CallControlScope::class.java.classLoader, arrayOf(CallControlScope::class.java)
  ) { proxy, method, args ->
    when (method.name) {
      "getAvailableEndpoints" -> endpoints()
      "requestEndpointChange" -> change()
      "getCoroutineContext" -> EmptyCoroutineContext
      "hashCode" -> System.identityHashCode(proxy)
      "equals" -> proxy === args?.get(0)
      "toString" -> "TelecomCall"
      else -> throw UnsupportedOperationException(method.name)
    }
  } as CallControlScope
}

@RunWith(MediaLayerTestRunner::class)
class AudioContractTest {
  private val volume = SteppedVolume(0.8f)
  private val app = AppPath(volume = volume)
  private val scope = CoroutineScope(SupervisorJob())

  @After fun close() {
    scope.cancel()
    app.close()
  }

  private fun <T> record(flow: Flow<T>): SignallingList<T> = SignallingList<T>().also { list -> scope.launch(Dispatchers.Unconfined) { flow.collect { list.add(it) } } }

  private suspend fun <T> Flow<T>.next(what: (T) -> Boolean): T = withTimeout(5_000) { first(what) }

  private suspend fun ended(call: MediaCallAudio) = call.states.next { it == CallState.ENDED }

  private suspend fun availability(value: MicrophoneAvailability) = app.layer.microphoneAvailability.next { it == value }

  @Test
  fun inv_call_15_a_managed_call_shows_live_and_interrupted_as_the_system_activates_it() = runBlocking {
    val call = app.layer.callAudio(MediaCallKind.Telecom(TelecomEndpoints()))
    val states = record(call.states)

    call.systemActivated()
    call.states.next { it == CallState.LIVE }
    call.systemDeactivated()
    call.states.next { it == CallState.INTERRUPTED }
    call.systemActivated()
    call.states.next { it == CallState.LIVE }

    assertEquals(listOf(CallState.CONNECTING, CallState.LIVE, CallState.INTERRUPTED, CallState.LIVE), states.toList())
    call.close()
  }

  @Test
  fun inv_call_16_every_loss_of_the_call_sound_shows_interrupted_before_a_new_live() = runBlocking {
    val rows = listOf<Pair<String, (MediaCallAudio) -> Unit>>(
      "transient focus loss" to { _ -> app.focus.send(FocusSignal.Change(FocusChange.LOSS_TRANSIENT)) },
      "permanent focus loss" to { _ -> app.focus.send(FocusSignal.Change(FocusChange.LOSS)) },
      "failed apply" to { call -> app.focus.requestFailure = SecurityException("focus denied"); call.setSpeaker(true) },
      "external call" to { _ -> app.focus.send(FocusSignal.PhoneCall(true)) }
    )
    val seen = rows.map { (name, cause) ->
      val call = app.layer.callAudio(MediaCallKind.Unmanaged)
      val states = record(call.states)
      call.states.next { it == CallState.LIVE }

      cause(call)

      call.states.next { it == CallState.INTERRUPTED }
      val shown = name to states.toList()
      app.focus.requestFailure = null
      app.focus.send(FocusSignal.PhoneCall(false))
      call.close()
      ended(call)
      shown
    }
    assertEquals(rows.map { it.first to listOf(CallState.CONNECTING, CallState.LIVE, CallState.INTERRUPTED) }, seen)
  }

  @Test
  fun inv_call_17_the_speaker_choice_of_every_call_kind_shows_its_applied_route() = runBlocking {
    val unmanaged = app.layer.callAudio(MediaCallKind.Unmanaged)
    assertEquals("a new call starts on the receiver", CallRoute.RECEIVER, unmanaged.route.next { true })
    unmanaged.states.next { it == CallState.LIVE }
    unmanaged.setSpeaker(true)
    unmanaged.route.next { it == CallRoute.SPEAKER }
    unmanaged.setSpeaker(false)
    unmanaged.route.next { it == CallRoute.RECEIVER }
    app.focus.communicationFailure = IllegalStateException("no communication device")
    unmanaged.setSpeaker(true)
    unmanaged.route.next { it == CallRoute.FAILED }
    unmanaged.close()
    ended(unmanaged)
    app.focus.communicationFailure = null

    val endpoints = TelecomEndpoints()
    val telecom = app.layer.callAudio(MediaCallKind.Telecom(endpoints))
    telecom.systemActivated()
    telecom.setSpeaker(true)
    telecom.route.next { it == CallRoute.SPEAKER }
    endpoints.accepts = false
    telecom.setSpeaker(false)
    telecom.route.next { it == CallRoute.FAILED }

    assertEquals(listOf(true, false), endpoints.asked.toList())
    telecom.close()
  }

  @Test
  fun inv_call_18_an_ended_call_and_a_call_of_a_closed_layer_both_show_ended() = runBlocking {
    val closedByApp = app.layer.callAudio(MediaCallKind.Unmanaged)
    val closedByLayer = app.layer.callAudio(MediaCallKind.Telecom(TelecomEndpoints()))

    closedByApp.close()
    ended(closedByApp)
    app.close()

    assertEquals(CallState.ENDED, ended(closedByLayer))
  }

  @Test
  fun inv_audio_59_a_permission_change_in_the_foreground_shows_in_the_availability() = runBlocking {
    availability(MicrophoneAvailability.AVAILABLE)

    app.permission.change(false)
    availability(MicrophoneAvailability.PERMISSION_DENIED)
    app.permission.change(true)
    availability(MicrophoneAvailability.AVAILABLE)
    Unit
  }

  @Test
  fun inv_audio_60_another_app_recording_shows_in_the_availability() = runBlocking {
    app.focus.send(FocusSignal.OtherRecording(true))
    availability(MicrophoneAvailability.OTHER_APP)
    app.focus.send(FocusSignal.OtherRecording(false))
    availability(MicrophoneAvailability.AVAILABLE)
    Unit
  }

  @Test
  fun inv_audio_61_a_delayed_or_refused_recording_session_is_not_available_until_it_comes_back() = runBlocking {
    val recorder = MediaVoiceRecorder(capture = WritingVoiceCapture(), maxDurationMs = 60_000u)
    app.focus.answer = FocusAnswer.DELAYED
    val delayed = assertThrows(MediaException.Voice::class.java) { runBlocking { recorder.start(File.createTempFile("voice", ".m4a").also { it.delete() }) } }
    assertEquals("a delayed recording session refuses the start", VoiceRefusal.MICROPHONE_BUSY, delayed.reason)
    app.layer.microphoneAvailability.next { it != MicrophoneAvailability.AVAILABLE }
    app.focus.answer = FocusAnswer.GRANTED
    app.focus.send(FocusSignal.Change(FocusChange.GAIN))
    availability(MicrophoneAvailability.AVAILABLE)
    recorder.start(File.createTempFile("voice", ".m4a").also { it.delete() })
    org.robolectric.shadows.ShadowSystemClock.advanceBy(java.time.Duration.ofSeconds(1))
    recorder.stop()

    app.foreground(false)
    app.focus.answer = FocusAnswer.FAILED
    assertThrows(MediaException.Voice::class.java) { runBlocking { recorder.start(File.createTempFile("voice", ".m4a").also { it.delete() }) } }
    app.layer.microphoneAvailability.next { it != MicrophoneAvailability.AVAILABLE }
    app.focus.answer = FocusAnswer.GRANTED
    app.foreground(true)
    availability(MicrophoneAvailability.AVAILABLE)
    Unit
  }

  @Test
  fun inv_audio_62_closing_the_layer_ends_the_availability_stream_and_a_later_stream_ends_at_once() = runBlocking {
    val stream = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { app.layer.microphoneAvailability.toList() }

    app.close()

    withTimeout(5_000) { stream.await() }
    assertEquals(emptyList<MicrophoneAvailability>(), withTimeout(5_000) { app.layer.microphoneAvailability.toList() })
  }

  @Test
  fun inv_failures_05_every_native_audio_failure_is_one_ordered_event_with_its_source() = runBlocking {
    val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.take(2).toList() }
    app.focus.requestFailure = SecurityException("focus denied")
    val first = app.layer.callAudio(MediaCallKind.Unmanaged)
    first.close()
    ended(first)
    app.focus.requestFailure = null
    app.focus.communicationFailure = IllegalStateException("no communication device")

    app.layer.callAudio(MediaCallKind.Unmanaged)

    assertEquals(listOf(
      NativeFailure(FailureSource.FOCUS, "java.lang.SecurityException", 0L, "focus denied"),
      NativeFailure(FailureSource.SPEAKER, "java.lang.IllegalStateException", 0L, "no communication device")
    ), withTimeout(5_000) { failures.await() })
  }

  @Test
  fun inv_audio_63_a_high_low_high_burst_behind_a_slow_collector_is_one_rise() {
    val worker = Executors.newSingleThreadExecutor()
    val busy = CountDownLatch(1)
    val afterBurst = AtomicBoolean()
    val rises = CopyOnWriteArrayList<Boolean>()
    try {
      scope.launch(worker.asCoroutineDispatcher()) { app.layer.volumeRaises().collect { rises.add(afterBurst.get()); TestSignal.fire() } }
      TestSignal.await("observing", 5_000) { volume.observers == 1 }
      worker.execute { busy.await() }

      fun changeVolume(value: Float) {
        val done = CountDownLatch(1)
        checkNotNull(app.layer.binding).link.handler.post { volume.set(value); done.countDown() }
        check(done.await(5, TimeUnit.SECONDS))
      }
      changeVolume(0.5f)
      changeVolume(0.8f)
      busy.countDown()
      TestSignal.await("the burst rise", 5_000) { rises.size == 1 }
      afterBurst.set(true)
      changeVolume(0.4f)
      changeVolume(0.9f)
      TestSignal.await("the next rise", 5_000) { rises.size == 2 }

      assertEquals(listOf(false, true), rises.toList())
    } finally {
      busy.countDown()
      worker.shutdownNow()
    }
  }

  @Test
  fun inv_audio_64_the_global_mute_reaches_a_layer_opened_after_it() = runBlocking {
    val url = "file:///muted.mp3"
    Media.muted = true
    try {
      TestSignal.await("the core took the global mute", 5_000) { Media.muted }
      app.close()
      AppPath().use { again ->
        val player = again.layer.playUrl(MediaItem(Kind.AUDIO, url, emptyMap(), null, null, strategy = CacheStrategy.WINDOW))

        player.play()

        TestSignal.await("the new layer plays muted", 5_000) { again.slots.playing(url) && again.slots.muted(url) }
        player.close()
      }
    } finally {
      Media.muted = false
      TestSignal.await("the global mute is off again", 5_000) { !Media.muted }
    }
  }

  @Test
  fun inv_audio_65_closing_the_layer_ends_the_volume_streams_and_releases_the_system_observer() = runBlocking {
    val raises = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { app.layer.volumeRaises().toList() }
    TestSignal.await("observing", 5_000) { volume.observers == 1 }

    app.close()

    withTimeout(5_000) { raises.await() }
    TestSignal.await("the system observer is released", 5_000) { volume.observers == 0 }
  }

  @Test
  fun inv_call_19_a_slow_collector_gets_every_call_transition_and_both_call_streams_end() = runBlocking {
    val call = app.layer.callAudio(MediaCallKind.Telecom(TelecomEndpoints()))
    val shown = SignallingList<CallState>()
    val watch = call.onState { shown.add(it) }
    val worker = Executors.newSingleThreadExecutor()
    val held = CountDownLatch(1)
    try {
      val collected = SignallingList<CallState>()
      var holding = true
      val states = scope.async(worker.asCoroutineDispatcher()) {
        call.states.collect {
          collected.add(it)
          if (it == CallState.LIVE && holding) { holding = false; held.await() }
        }
        collected.toList()
      }
      val routeSeen = SignallingList<CallRoute>()
      val routes = scope.async(Dispatchers.Default) { call.route.collect { routeSeen.add(it) }; routeSeen.toList() }
      TestSignal.await("both streams observe the call", 5_000) { collected.isNotEmpty() && routeSeen.isNotEmpty() }

      call.systemActivated()
      TestSignal.await("the collector holds after live", 5_000) { collected.lastOrNull() == CallState.LIVE }
      call.systemDeactivated()
      TestSignal.await("interrupted while the collector holds", 5_000) { shown.lastOrNull() == CallState.INTERRUPTED }
      call.systemActivated()
      TestSignal.await("live again while the collector holds", 5_000) { shown.count { it == CallState.LIVE } == 2 }
      call.close()
      TestSignal.await("ended while the collector holds", 5_000) { shown.lastOrNull() == CallState.ENDED }
      held.countDown()

      assertEquals(listOf(CallState.CONNECTING, CallState.LIVE, CallState.INTERRUPTED, CallState.LIVE, CallState.ENDED), withTimeout(5_000) { states.await() })
      assertEquals(listOf(CallRoute.RECEIVER), withTimeout(5_000) { routes.await() })
    } finally {
      held.countDown()
      watch.close()
      worker.shutdownNow()
    }
  }

  @Test
  fun inv_call_20_a_refused_telecom_speaker_change_shows_the_failure_of_telecom_itself() = runBlocking {
    val refused = CallControlResult.Error(3)
    val endpoints = flowOf(listOf(
      CallEndpointCompat("Speaker", CallEndpointCompat.TYPE_SPEAKER, ParcelUuid(UUID.randomUUID())),
      CallEndpointCompat("Earpiece", CallEndpointCompat.TYPE_EARPIECE, ParcelUuid(UUID.randomUUID()))
    ))
    val rows = listOf<Pair<Flow<List<CallEndpointCompat>>, NativeFailure>>(
      endpoints to NativeFailure(FailureSource.SPEAKER, "androidx.core.telecom", 3L, refused.toString()),
      flow<List<CallEndpointCompat>> { throw IllegalStateException("telecom service gone") } to
        NativeFailure(FailureSource.SPEAKER, "java.lang.IllegalStateException", 0L, "telecom service gone")
    )
    var current = rows.first().first
    val telecom = TelecomCall({ current }, { refused })
    val call = app.layer.callAudio(MediaCallKind.Telecom(CallEndpointPort.of(telecom.scope)))
    call.systemActivated()
    call.states.next { it == CallState.LIVE }
    val failures = SignallingList<NativeFailure>()
    val watch = scope.launch(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.collect { failures.add(it) } }

    rows.forEachIndexed { index, (shown, _) ->
      current = shown
      call.setSpeaker(index % 2 == 0)
      TestSignal.await("the failure of request $index", 5_000) { failures.size == index + 1 }
    }

    watch.cancel()
    assertEquals(rows.map { it.second }, failures.toList())
    assertEquals(CallRoute.FAILED, call.route.next { true })
    call.close()
  }

  @Test
  fun inv_audio_67_a_focus_release_the_system_refuses_is_one_failure_event_with_the_code_of_the_system() = runBlocking {
    app.close()
    val context = ApplicationProvider.getApplicationContext<Context>()
    val manager = shadowOf(context.getSystemService(AudioManager::class.java))
    val rows = listOf<Pair<String, suspend (AppPath, MediaVoiceRecorder) -> Unit>>(
      "the recording ends" to { _, recorder -> recorder.stop() },
      "the layer closes during the recording" to { path, _ -> path.close() }
    )
    val refused = NativeFailure(FailureSource.FOCUS, "android.media.AudioManager", AudioManager.AUDIOFOCUS_REQUEST_FAILED.toLong(), "Audio focus abandon refused")
    try {
      val seen = rows.map { (name, end) ->
        manager.setNextFocusRequestResponse(AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
        AppPath(ports = AudioPorts(AudioSystem(context), TogglePermission())).use { path ->
          val recorder = MediaVoiceRecorder(capture = WritingVoiceCapture(), maxDurationMs = 60_000u)
          recorder.start(File.createTempFile("voice", ".m4a").also { it.delete() })
          org.robolectric.shadows.ShadowSystemClock.advanceBy(java.time.Duration.ofSeconds(1))
          val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.first { it.source == FailureSource.FOCUS } }
          manager.setNextFocusRequestResponse(AudioManager.AUDIOFOCUS_REQUEST_FAILED)

          end(path, recorder)

          val shown = name to withTimeout(5_000) { failures.await() }
          manager.setNextFocusRequestResponse(AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
          path.closeAndAwait()
          shown
        }
      }
      assertEquals(rows.map { it.first to refused }, seen)
    } finally {
      manager.setNextFocusRequestResponse(AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
    }
  }
}

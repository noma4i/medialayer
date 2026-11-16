package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import com.noma4i.medialayer.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flow

enum class MediaCallKind { Telecom, Unmanaged }

internal enum class FocusGain { GAIN, GAIN_TRANSIENT, GAIN_TRANSIENT_EXCLUSIVE }
internal enum class FocusUsage { MEDIA, VOICE_COMMUNICATION }
internal data class FocusPlan(val gain: FocusGain, val usage: FocusUsage, val communication: Boolean) {
  companion object {
    fun of(want: SessionWant): FocusPlan? = when (want) {
      is SessionWant.Playback -> if (want.focus == AudioFocus.MIX) null else FocusPlan(FocusGain.GAIN, FocusUsage.MEDIA, false)
      is SessionWant.Recording -> FocusPlan(FocusGain.GAIN_TRANSIENT_EXCLUSIVE, FocusUsage.MEDIA, false)
      is SessionWant.Call -> FocusPlan(FocusGain.GAIN_TRANSIENT, FocusUsage.VOICE_COMMUNICATION, true)
    }
  }
}

internal sealed interface FocusSignal {
  data class Change(val value: FocusChange) : FocusSignal
  data object Noisy : FocusSignal
  data class CommunicationMode(val active: Boolean) : FocusSignal
  data class PhoneCall(val active: Boolean) : FocusSignal
  data class OtherRecording(val active: Boolean) : FocusSignal
  data class Silenced(val id: ULong) : FocusSignal
}

internal interface AudioFocusPort {
  fun listen(sink: (FocusSignal) -> Unit)
  fun request(plan: FocusPlan): FocusAnswer
  fun abandon()
  fun setCommunication(on: Boolean, speaker: Boolean)
}
internal interface MicrophonePermissionPort { fun granted(): Boolean }
internal data class AudioPorts(val focus: AudioFocusPort, val permission: MicrophonePermissionPort)
internal sealed interface Admission {
  data class Admitted(val ticket: RecordingTicket) : Admission
  data class Refused(val reason: RecordingRefusal) : Admission
}

internal class RecordingTicket(val id: ULong, private val end: (ULong) -> Unit) : Closeable {
  private val closed = AtomicBoolean()
  private val listeners = LinkedHashSet<(RecordingRefusal) -> Unit>()
  private var reason: RecordingRefusal? = null
  fun onRevoked(listener: (RecordingRefusal) -> Unit): Closeable {
    synchronized(listeners) { listeners.add(listener); reason?.let(listener) }
    return Closeable { synchronized(listeners) { listeners.remove(listener) } }
  }
  internal fun revoke(value: RecordingRefusal) = synchronized(listeners) {
    reason = value
    listeners.toList().forEach { it(value) }
  }
  override fun close() { if (closed.compareAndSet(false, true)) end(id) }
}

class MediaCallAudio internal constructor(private val id: ULong, private val send: (AudioInput) -> Unit) : Closeable {
  private val closed = AtomicBoolean()
  private val state = MutableStateFlow(CallState.CONNECTING)
  private val listeners = LinkedHashSet<(CallState) -> Unit>()
  val states: StateFlow<CallState> = state
  fun onState(listener: (CallState) -> Unit): Closeable {
    synchronized(listeners) { listeners.add(listener); listener(state.value) }
    return Closeable { synchronized(listeners) { listeners.remove(listener) } }
  }
  internal fun update(value: CallState) = synchronized(listeners) {
    if (state.value != value) { state.value = value; listeners.toList().forEach { it(value) } }
  }
  fun setSpeaker(on: Boolean) { if (!closed.get()) send(AudioInput.CallSpeaker(id, on)) }
  fun systemActivated() { if (!closed.get()) send(AudioInput.CallSystemActivated(id)) }
  fun systemDeactivated() { if (!closed.get()) send(AudioInput.CallSystemDeactivated(id)) }
  override fun close() { if (closed.compareAndSet(false, true)) send(AudioInput.CallEnded(id)) }
}

internal class AudioLink(core: MediaLayer, private val ports: AudioPorts, private val volume: VolumePort? = null) : Closeable {
  private val audioInputs = core.audioInputs()
  private val thread = HandlerThread("MediaLayer audio").also { it.start() }
  internal val handler = Handler(thread.looper)
  private val nextId = AtomicLong()
  private val closed = AtomicBoolean()
  private val postLock = Any()
  private val pending = HashMap<ULong, CompletableDeferred<Admission>>()
  private val tickets = HashMap<ULong, RecordingTicket>()
  private val recordingCount = AtomicInteger()
  internal val isRecording: Boolean get() = recordingCount.get() != 0
  private val calls = HashMap<ULong, MediaCallAudio>()
  private val availability = MutableStateFlow(MicrophoneAvailability.AVAILABLE)
  val microphoneAvailability: StateFlow<MicrophoneAvailability> = availability
  private val listeners = LinkedHashSet<(MicrophoneAvailability) -> Unit>()
  private var communication = false
  private val inputs = ArrayDeque<AudioInput>()
  private var processing = false
  @Volatile internal var volumeMultiplier: (Float) -> Unit = {}

  init {
    (ports.focus as? AudioSystem)?.handler = handler
    ports.focus.listen { signal ->
      send(when (signal) {
        is FocusSignal.Change -> AudioInput.Focus(signal.value)
        FocusSignal.Noisy -> AudioInput.Noisy
        is FocusSignal.CommunicationMode -> AudioInput.CommunicationMode(signal.active)
        is FocusSignal.PhoneCall -> AudioInput.PhoneCall(signal.active)
        is FocusSignal.OtherRecording -> AudioInput.OtherRecording(signal.active)
        is FocusSignal.Silenced -> AudioInput.Silenced(signal.id)
      })
    }
    send(AudioInput.Permission(ports.permission.granted()))
  }
  private fun id() = nextId.incrementAndGet().toULong()
  private fun enqueue(work: () -> Unit): Boolean = synchronized(postLock) { !closed.get() && handler.post(work) }
  internal fun send(input: AudioInput) { enqueue { process(input) } }
  private fun process(input: AudioInput) {
    inputs.addLast(input)
    if (processing) return
    processing = true
    try { while (inputs.isNotEmpty()) applyInput(inputs.removeFirst()) } finally { processing = false }
  }
  private fun applyInput(input: AudioInput) {
    val actions = try { audioInputs.send(input) } catch (failure: MediaException) {
      pending.values.forEach { it.completeExceptionally(failure) }
      pending.clear()
      return
    }
    actions.forEach { action ->
      when (action) {
        is AudioAction.Apply -> {
          try {
            val plan = FocusPlan.of(action.want)
            if (plan != null) {
              val answer = ports.focus.request(plan)
              if (plan.communication) {
                ports.focus.setCommunication(true, (action.want as SessionWant.Call).speaker)
                communication = true
              }
              if (answer == FocusAnswer.GRANTED) process(AudioInput.Applied(action.generation, AppliedResult.Ok))
              else process(AudioInput.FocusRequest(answer))
            } else {
              process(AudioInput.Applied(action.generation, AppliedResult.Ok))
            }
          } catch (failure: Exception) { process(AudioInput.Applied(action.generation, AppliedResult.Failed(-1))) }
        }
        is AudioAction.Deactivate -> {
          try {
            if (communication) { ports.focus.setCommunication(false, false); communication = false }
            ports.focus.abandon()
            process(AudioInput.Applied(action.generation, AppliedResult.Ok))
          } catch (failure: Exception) { process(AudioInput.Applied(action.generation, AppliedResult.Failed(-1))) }
        }
        is AudioAction.Admit -> pending.remove(action.id)?.complete(Admission.Admitted(RecordingTicket(action.id, ::endTicket).also {
          tickets[action.id] = it
          recordingCount.incrementAndGet()
          (ports.focus as? AudioSystem)?.ownRecording(true)
        }))
        is AudioAction.Refuse -> pending.remove(action.id)?.complete(Admission.Refused(action.reason))
        is AudioAction.Revoke -> tickets[action.id]?.revoke(action.reason)
        is AudioAction.Availability -> synchronized(listeners) { availability.value = action.value; listeners.toList().forEach { it(action.value) } }
        is AudioAction.CallState -> {
          calls[action.id]?.update(action.state)
          if (action.state == CallState.ENDED) calls.remove(action.id)
        }
        is AudioAction.VolumeMultiplier -> volumeMultiplier(action.value)
        else -> Unit
      }
    }
  }
  fun onAvailability(listener: (MicrophoneAvailability) -> Unit): Closeable {
    synchronized(listeners) { listeners.add(listener); listener(availability.value) }
    return Closeable { synchronized(listeners) { listeners.remove(listener) } }
  }
  suspend fun admit(kind: RecordingKind): Admission {
    val id = id()
    val result = CompletableDeferred<Admission>()
    if (!enqueue { pending[id] = result; process(AudioInput.Capture(id, kind)) }) throw MediaException.Closed()
    try { return result.await() } catch (failure: Throwable) {
      enqueue {
        pending.remove(id)
        val ticket = tickets.remove(id)
        if (ticket == null) process(AudioInput.CaptureEnded(id)) else ticket.close()
      }
      throw failure
    }
  }
  private fun endTicket(id: ULong) { recordingCount.decrementAndGet(); enqueue {
    tickets.remove(id)
    process(AudioInput.CaptureEnded(id))
    if (recordingCount.get() == 0) (ports.focus as? AudioSystem)?.ownRecording(false)
  } }
  fun call(kind: MediaCallKind): MediaCallAudio {
    val id = id()
    val call = MediaCallAudio(id, ::send)
    val coreKind = when (kind) { MediaCallKind.Telecom -> CallKind.Telecom; MediaCallKind.Unmanaged -> CallKind.Unmanaged }
    if (!enqueue { calls[id] = call; process(AudioInput.CallStarted(id, coreKind, false)) }) throw MediaException.Closed()
    return call
  }
  fun audible(slot: ULong, on: Boolean) = send(AudioInput.Audible(slot, on))
  fun foreground(on: Boolean) { if (on) { (ports.focus as? AudioSystem)?.refreshPhone(); send(AudioInput.Permission(ports.permission.granted())) }; send(AudioInput.Foreground(on)) }
  fun readVolume(): MediaVolumeReading? {
    val reading = volume?.reading() ?: return null
    return MediaVolumeReading(reading.initial, withVolumeRead(reading.changes))
  }
  fun volumeRaises(): Flow<Unit> = volume?.raises()?.let(::withVolumeRead) ?: emptyFlow()
  private fun <T> withVolumeRead(source: Flow<T>): Flow<T> = flow {
    val id = id()
    send(AudioInput.VolumeReadStarted(id))
    try { source.collect { emit(it) } } finally { send(AudioInput.VolumeReadEnded(id)) }
  }
  override fun close() {
    val done = CountDownLatch(1)
    val finish: () -> Unit = {
      try {
        pending.values.forEach { it.completeExceptionally(MediaException.Closed()) }
        pending.clear()
        if (communication) ports.focus.setCommunication(false, false)
        ports.focus.abandon()
        (ports.focus as? AutoCloseable)?.close()
        thread.quitSafely()
      } finally { done.countDown() }
    }
    val onThread = Looper.myLooper() == thread.looper
    synchronized(postLock) {
      if (!closed.compareAndSet(false, true)) return
      if (!onThread) check(handler.post(finish))
    }
    if (onThread) finish() else check(done.await(5, TimeUnit.SECONDS))
  }
}

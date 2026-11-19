package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallException
import androidx.core.telecom.CallEndpointCompat
import com.noma4i.medialayer.*
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.transformWhile
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.asCoroutineDispatcher

interface CallEndpointPort {
  fun select(speaker: Boolean, done: (CallEndpointResult) -> Unit)
  companion object { fun of(scope: CallControlScope): CallEndpointPort = TelecomEndpoint(scope) }
}

sealed interface CallEndpointResult {
  data object Applied : CallEndpointResult
  data class Failed(val failure: NativeFailure) : CallEndpointResult
}

sealed interface MediaCallKind {
  data object Unmanaged : MediaCallKind
  class Telecom(val control: CallEndpointPort) : MediaCallKind
}

private class TelecomEndpoint(private val call: CallControlScope) : CallEndpointPort {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  override fun select(speaker: Boolean, done: (CallEndpointResult) -> Unit) {
    scope.launch {
      val applied = try {
        val type = if (speaker) CallEndpointCompat.TYPE_SPEAKER else CallEndpointCompat.TYPE_EARPIECE
        val endpoint = call.availableEndpoints.first().firstOrNull { it.type == type } ?: error("Endpoint unavailable")
        when (val result = call.requestEndpointChange(endpoint)) {
          is CallControlResult.Success -> CallEndpointResult.Applied
          is CallControlResult.Error -> CallEndpointResult.Failed(NativeFailure(FailureSource.SPEAKER, "androidx.core.telecom", result.errorCode.toLong(), result.toString()))
        }
      } catch (error: CancellationException) { throw error } catch (error: Exception) {
        CallEndpointResult.Failed(NativeFailure(FailureSource.SPEAKER, error.javaClass.name, (error as? CallException)?.code?.toLong() ?: 0L, error.message ?: error.toString()))
      }
      done(applied)
    }
  }
}

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
  fun enterCommunication(speaker: Boolean)
  fun leaveCommunication()
}
internal interface MicrophonePermissionPort {
  fun state(): MicrophonePermission
  fun listen(sink: (MicrophonePermission) -> Unit) = Unit
}
internal data class AudioPorts(val focus: AudioFocusPort, val permission: MicrophonePermissionPort)
internal sealed interface Admission {
  data class Admitted(val ticket: RecordingTicket) : Admission
  data class Refused(val reason: RecordingRefusal) : Admission
}

internal class RecordingTicket(val id: ULong, private val end: (ULong) -> Unit, private val take: (ULong, Boolean) -> Unit = { _, _ -> }) : Closeable {
  private val closed = AtomicBoolean()
  /** A camera take begins or ends: the core refuses a voice during a take and gives the voice the microphone of an idle preview. */
  fun taking(on: Boolean) { if (!closed.get()) take(id, on) }
  private val gates = ArrayList<suspend () -> Unit>()
  private val ended = CompletableDeferred<Unit>()
  private val gateScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  /** The ticket ends only after [idle] returns: a microphone of this recording still holds the system's AudioRecord until then, and the system would count it as another app's recording. */
  fun gate(idle: suspend () -> Unit) { synchronized(gates) { gates.add(idle) } }
  private val listeners = LinkedHashSet<(RecordingRefusal, () -> Unit) -> Unit>()
  private var reason: RecordingRefusal? = null
  private val stopped = CompletableDeferred<Unit>()
  fun onRevoked(listener: (RecordingRefusal, () -> Unit) -> Unit): Closeable {
    val existing = synchronized(listeners) { listeners.add(listener); reason }
    existing?.let { listener(it) {} }
    return Closeable { synchronized(listeners) { listeners.remove(listener) } }
  }
  internal suspend fun revoke(value: RecordingRefusal) {
    val notify = synchronized(listeners) {
      if (reason != null) return@synchronized null
      reason = value
      listeners.toList()
    }
    if (notify != null) {
      val acknowledgements = notify.map { listener ->
        CompletableDeferred<Unit>().also { done -> listener(value) { done.complete(Unit) } }
      }
      acknowledgements.forEach { it.await() }
      stopped.complete(Unit)
    }
    stopped.await()
  }
  fun start(work: () -> Unit): RecordingRefusal? = synchronized(listeners) {
    reason?.let { return@synchronized it }
    work()
    null
  }
  override fun close() {
    if (!closed.compareAndSet(false, true)) return
    val waits = synchronized(gates) { gates.toList() }
    if (waits.isEmpty()) { end(id); ended.complete(Unit); return }
    gateScope.launch { try { waits.forEach { it() } } finally { end(id); ended.complete(Unit) } }
  }

  /** Closes the ticket and returns when it has ended, after the microphones of the recording are released. */
  suspend fun closeAndWait() { close(); ended.await() }
}

class MediaCallAudio internal constructor(private val id: ULong, internal val kind: MediaCallKind, private val send: (AudioInput) -> Unit) : Closeable {
  private val closed = AtomicBoolean()
  private val awake = ScreenAwakeHold().also { it.set(true) }
  private val state = MutableStateFlow(CallState.CONNECTING)
  private inner class StateObservation(var listener: ((CallState) -> Unit)?) : Closeable {
    override fun close() = synchronized(listeners) { listener = null; listeners.remove(this); Unit }
  }
  private val listeners = LinkedHashSet<StateObservation>()
  val states: Flow<CallState> = kotlinx.coroutines.flow.callbackFlow {
    val observation = onState { trySend(it); if (it == CallState.ENDED) channel.close() }
    awaitClose { observation.close() }
  }.buffer(Channel.UNLIMITED)
  private val currentRoute = MutableStateFlow(CallRoute.RECEIVER)
  val route: Flow<CallRoute> = kotlinx.coroutines.flow.callbackFlow {
    val job = launch { currentRoute.collect { trySend(it) } }
    val observation = onState { if (it == CallState.ENDED) channel.close() }
    awaitClose { job.cancel(); observation.close() }
  }
  internal fun route(value: CallRoute) { currentRoute.value = value }
  fun onState(listener: (CallState) -> Unit): Closeable = synchronized(listeners) {
    if (state.value == CallState.ENDED) {
      listener(CallState.ENDED)
      return@synchronized Closeable { }
    }
    val observation = StateObservation(listener)
    listeners.add(observation)
    listener(state.value)
    observation
  }
  internal fun update(value: CallState) = synchronized(listeners) {
    if (state.value == CallState.ENDED || state.value == value) return@synchronized
    state.value = value
    val notify = listeners.mapNotNull { it.listener }
    if (value == CallState.ENDED) {
      awake.set(false)
      listeners.forEach { it.listener = null }
      listeners.clear()
    }
    notify.forEach { it(value) }
  }
  fun setSpeaker(on: Boolean) { if (!closed.get()) send(AudioInput.CallSpeaker(id, on)) }
  fun systemActivated() { if (!closed.get()) send(AudioInput.CallSystemActivated(id)) }
  fun systemDeactivated() { if (!closed.get()) send(AudioInput.CallSystemDeactivated(id)) }
  override fun close() { if (closed.compareAndSet(false, true)) { awake.set(false); send(AudioInput.CallEnded(id)) } }
}

internal interface AudioInputOwner {
  fun restore(input: AudioInputHandle, generation: ULong)
  fun stop(generation: ULong)
  /** Returns once the microphone of this owner holds no AudioRecord: the system has it back. */
  suspend fun idle() = Unit
}

internal class AudioInputHandle(internal val id: ULong, private val link: AudioLink) : AutoCloseable {
  fun sound(generation: ULong, heard: Boolean) = link.send(AudioInput.NoSound(id, generation, !heard))
  fun restoreFailed(generation: ULong) = link.send(AudioInput.RestoreFailed(id, generation))
  override fun close() = link.closeInput(id)
}

private class InputRegistration(owner: AudioInputOwner, handle: AudioInputHandle) {
  val owner = java.lang.ref.WeakReference(owner)
  val handle = java.lang.ref.WeakReference(handle)
  var generation: ULong? = null

  fun stopLive() {
    val live = generation ?: return
    generation = null
    owner.get()?.stop(live)
  }
}

internal class AudioLink(core: MediaLayer, private val ports: AudioPorts, private val volume: VolumePort? = null) : Closeable {
  private val audioInputs = core.audioInputs()
  private val core = java.lang.ref.WeakReference(core)
  private val thread = HandlerThread("MediaLayer audio").also { it.start() }
  internal val handler = Handler(thread.looper)
  private val audioDispatcher = java.util.concurrent.Executor { handler.post(it) }.asCoroutineDispatcher()
  private val audioScope = CoroutineScope(SupervisorJob() + audioDispatcher)
  private val nextId = AtomicLong()
  private val closed = AtomicBoolean()
  internal val isClosed: Boolean get() = closed.get()
  internal val acceptsMute: Boolean get() = !closed.get() && core.get()?.uniffiIsDestroyed == false
  private val postLock = Any()
  private val pending = HashMap<ULong, CompletableDeferred<Admission>>()
  private val tickets = HashMap<ULong, RecordingTicket>()
  private val inputRegistry = HashMap<ULong, InputRegistration>()
  private val retiredInputs = ArrayList<java.lang.ref.WeakReference<AudioInputOwner>>()
  private val recordingCount = AtomicInteger()
  internal val isRecording: Boolean get() = recordingCount.get() != 0
  private val calls = LinkedHashMap<ULong, MediaCallAudio>()
  private val closeListeners = LinkedHashSet<() -> Unit>()
  private val closeDone = CompletableDeferred<Unit>()
  private var closeCompleted = false
  private val availability = MutableStateFlow(MicrophoneAvailability.AVAILABLE)
  val microphoneAvailability: Flow<MicrophoneAvailability> = kotlinx.coroutines.flow.callbackFlow {
    if (isClosed) channel.close()
    val close = onClose { channel.close() }
    val observation = onAvailability { trySend(it) }
    awaitClose { observation.close(); close.close() }
  }
  private val listeners = LinkedHashSet<(MicrophoneAvailability) -> Unit>()
  private var communication = false
  private var focusHeld = false
  private val actions = ArrayDeque<AudioAction>()
  private var processing = false
  @Volatile internal var volumeMultiplier: (Float) -> Unit = {}

  init {
    (ports.focus as? AudioSystem)?.handler = handler
    (volume as? SystemVolume)?.handler = handler
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
    ports.permission.listen { send(AudioInput.Permission(it)) }
    send(AudioInput.Permission(ports.permission.state()))
  }
  private fun id() = nextId.incrementAndGet().toULong()
  private fun enqueue(work: () -> Unit): Boolean = synchronized(postLock) { !closed.get() && handler.post(work) }
  /** Feeds a test signal through the same inputs the system ports send. */
  @OptIn(MediaLayerTestingApi::class)
  internal fun drive(signal: AudioDriveSignal) {
    when (signal) {
      AudioDriveSignal.SilenceMicrophone -> enqueue {
        tickets.keys.toList().forEach { process(AudioInput.Silenced(it)) }
        inputRegistry.forEach { (id, input) -> input.generation?.let { process(AudioInput.NoSound(id, it, true)) } }
      }
      AudioDriveSignal.RestoreMicrophone -> enqueue {
        inputRegistry.forEach { (id, input) -> input.generation?.let { process(AudioInput.NoSound(id, it, false)) } }
      }
      AudioDriveSignal.StartCall -> send(AudioInput.PhoneCall(true))
      AudioDriveSignal.EndCall -> send(AudioInput.PhoneCall(false))
      AudioDriveSignal.BeginInterruption -> send(AudioInput.Focus(FocusChange.LOSS_TRANSIENT))
      AudioDriveSignal.EndInterruption -> send(AudioInput.Focus(FocusChange.GAIN))
      is AudioDriveSignal.Permission -> send(AudioInput.Permission(signal.state))
    }
  }
  internal fun send(input: AudioInput) {
    if (Looper.myLooper() == thread.looper && !closed.get()) process(input) else enqueue { process(input) }
  }
  fun openInput(kind: RecordingKind, owner: AudioInputOwner): AudioInputHandle {
    val handle = AudioInputHandle(id(), this)
    enqueue {
      inputRegistry[handle.id] = InputRegistration(owner, handle)
      process(AudioInput.InputOpened(handle.id, kind))
    }
    return handle
  }
  fun closeInput(id: ULong) {
    enqueue {
      inputRegistry.remove(id)?.let { it.stopLive(); retiredInputs.add(it.owner) }
      process(AudioInput.InputClosed(id))
    }
  }
  private fun process(input: AudioInput) {
    val next = try { audioInputs.send(input) } catch (failure: MediaException) {
      Media.recordFailure(failure)
      pending.values.forEach { it.completeExceptionally(failure) }
      pending.clear()
      return
    }
    actions.addAll(next)
    if (processing) return
    processing = true
    audioScope.launch(start = CoroutineStart.UNDISPATCHED) {
      try { while (!closed.get() && actions.isNotEmpty()) applyAction(actions.removeFirst()) } finally { processing = false }
    }
  }
  private suspend fun applyAction(action: AudioAction) {
      when (action) {
        is AudioAction.AwaitPaused -> audioInputs.paused(action.barrier)
        is AudioAction.Failure -> Media.recordNativeFailure(action.failure)
        is AudioAction.Route -> {
          val call = calls.values.lastOrNull()
          val endpoint = (call?.kind as? MediaCallKind.Telecom)?.control
          endpoint?.select(action.speaker) { applied ->
            call.route(if (applied is CallEndpointResult.Failed) CallRoute.FAILED else if (action.speaker) CallRoute.SPEAKER else CallRoute.RECEIVER)
            if (applied is CallEndpointResult.Failed) Media.recordNativeFailure(applied.failure)
          }
        }
        is AudioAction.Apply -> {
          var source = FailureSource.FOCUS
          try {
            val plan = FocusPlan.of(action.want)
            if (communication && plan?.communication != true) {
              try { ports.focus.leaveCommunication() } finally { communication = false }
            }
            if (plan == null && focusHeld) {
              ports.focus.abandon()
              focusHeld = false
            }
            if (plan != null) {
              val answer = ports.focus.request(plan)
              focusHeld = answer != FocusAnswer.FAILED
              if (plan.communication && focusHeld) {
                communication = true
                source = FailureSource.SPEAKER
                val speaker = (action.want as SessionWant.Call).speaker
                ports.focus.enterCommunication(speaker)
                calls.values.lastOrNull()?.route(if (speaker) CallRoute.SPEAKER else CallRoute.RECEIVER)
              } else if (communication && !focusHeld) {
                try { ports.focus.leaveCommunication() } finally { communication = false }
              }
              if (answer == FocusAnswer.GRANTED) process(AudioInput.Applied(action.generation, AppliedResult.Ok))
              else process(AudioInput.FocusRequest(generation = action.generation, answer = answer))
            } else {
              process(AudioInput.Applied(action.generation, AppliedResult.Ok))
            }
          } catch (failure: Exception) {
            if (failure is FocusAbandonFailure) reportFocusFailure(failure) else Media.recordNativeFailure(source, failure)
            if (source == FailureSource.SPEAKER) calls.values.lastOrNull()?.route(CallRoute.FAILED)
            process(AudioInput.Applied(action.generation, AppliedResult.Failed(-1)))
          }
        }
        is AudioAction.Deactivate -> {
          var abandoned = true
          try {
            ports.focus.abandon()
            focusHeld = false
          } catch (failure: Exception) { abandoned = false; reportFocusFailure(failure) }
          if (communication) {
            try { ports.focus.leaveCommunication() } catch (failure: Exception) { Media.recordNativeFailure(FailureSource.SPEAKER, failure) } finally { communication = false }
          }
          process(AudioInput.Applied(action.generation, if (abandoned) AppliedResult.Ok else AppliedResult.Failed(-1)))
        }
        is AudioAction.Admit -> pending.remove(action.id)?.complete(Admission.Admitted(RecordingTicket(action.id, ::endTicket) { id, on -> send(AudioInput.CaptureTake(id, on)) }.also {
          tickets[action.id] = it
          recordingCount.incrementAndGet()
          onTestChange?.invoke()
          (ports.focus as? AudioSystem)?.ownRecordings(recordingCount.get())
        }))
        is AudioAction.Refuse -> pending.remove(action.id)?.complete(Admission.Refused(action.reason))
        is AudioAction.Revoke -> tickets[action.id]?.revoke(action.reason)
        is AudioAction.RestoreInput -> inputRegistry[action.id]?.let { input ->
          // Android gives the microphone to an AudioRecord of the app only after the AudioRecord the core stopped is released: the input starts on that release event.
          val released = inputRegistry.values.filter { it !== input && it.generation == null }.mapNotNull { it.owner.get() } + retiredInputs.mapNotNull { it.get() }
          released.forEach { it.idle() }
          retiredInputs.removeAll { owner -> owner.get().let { it == null || it in released } }
          if (inputRegistry[action.id] !== input) return
          input.generation = action.generation
          input.handle.get()?.let { handle -> input.owner.get()?.restore(handle, action.generation) }
        }
        is AudioAction.StopInput -> inputRegistry[action.id]?.let { input ->
          input.generation = null
          input.owner.get()?.stop(action.generation)
        }
        is AudioAction.Availability -> synchronized(listeners) { availability.value = action.value; listeners.toList().forEach { it(action.value) } }
        is AudioAction.CallState -> {
          calls[action.id]?.update(action.state)
          if (action.state == CallState.ENDED) calls.remove(action.id)
        }
        is AudioAction.VolumeMultiplier -> volumeMultiplier(action.value)
        else -> Unit
      }
  }
  fun onAvailability(listener: (MicrophoneAvailability) -> Unit): Closeable {
    synchronized(listeners) { listeners.add(listener); listener(availability.value) }
    return Closeable { synchronized(listeners) { listeners.remove(listener) } }
  }
  private fun reportFocusFailure(failure: Throwable) {
    if (failure is FocusAbandonFailure) Media.recordNativeFailure(NativeFailure(FailureSource.FOCUS, "android.media.AudioManager", failure.code.toLong(), failure.message ?: "Audio focus abandon refused"))
    else Media.recordNativeFailure(FailureSource.FOCUS, failure)
  }
  suspend fun admit(kind: RecordingKind): Admission {
    val id = id()
    val result = CompletableDeferred<Admission>()
    if (!enqueue { pending[id] = result; process(AudioInput.Permission(ports.permission.state())); process(AudioInput.Capture(id, kind)) }) throw MediaException.Closed()
    try { return result.await() } catch (failure: Throwable) {
      enqueue {
        pending.remove(id)
        val ticket = tickets.remove(id)
        if (ticket == null) process(AudioInput.CaptureEnded(id)) else ticket.close()
      }
      throw failure
    }
  }
  private fun endTicket(id: ULong) { recordingCount.decrementAndGet(); onTestChange?.invoke(); enqueue {
    tickets.remove(id)
    process(AudioInput.CaptureEnded(id))
    (ports.focus as? AudioSystem)?.ownRecordings(recordingCount.get())
  } }
  fun call(kind: MediaCallKind): MediaCallAudio {
    val id = id()
    val call = MediaCallAudio(id, kind, ::send)
    val coreKind = when (kind) { is MediaCallKind.Telecom -> CallKind.Telecom; MediaCallKind.Unmanaged -> CallKind.Unmanaged }
    if (!enqueue { calls[id] = call; process(AudioInput.CallStarted(id, coreKind, false)) }) throw MediaException.Closed()
    return call
  }
  fun audible(slot: ULong, on: Boolean) = send(AudioInput.Audible(slot, on))
  fun setMuted(value: Boolean, done: (Boolean) -> Unit) {
    if (!enqueue {
      val accepted = try { (core.get() ?: throw MediaException.Closed()).setMutedAll(value); true } catch (error: Exception) { Media.recordNativeFailure(FailureSource.SESSION, error); false }
      done(accepted)
    }) done(false)
  }
  fun onClose(listener: () -> Unit): Closeable {
    synchronized(closeListeners) {
      if (closeCompleted) listener() else closeListeners.add(listener)
    }
    return Closeable { synchronized(closeListeners) { closeListeners.remove(listener) } }
  }
  suspend fun awaitClosed() { closeDone.await() }
  fun foreground(on: Boolean) { if (on) { (ports.focus as? AudioSystem)?.refreshPhone(); send(AudioInput.Permission(ports.permission.state())) }; send(AudioInput.Foreground(on)) }
  suspend fun readVolume(): MediaVolumeReading? {
    val port = volume ?: return null
    val initial = CompletableDeferred<Float?>()
    if (!enqueue { runCatching { port.level() }.fold(initial::complete, initial::completeExceptionally) }) return null
    val level = initial.await() ?: return null
    if (closed.get()) return null
    val reading = port.reading(level, handler)
    return MediaVolumeReading(reading.initial, withVolumeRead(reading.changes))
  }
  fun volumeRaises(): Flow<Unit> = flow {
    val reading = readVolume() ?: return@flow
    var previous = reading.initial
    reading.changes.transformWhile { current ->
      val result = CompletableDeferred<Boolean>()
      if (!enqueue { runCatching { isVolumeRaise(previous, current) }.fold(result::complete, result::completeExceptionally) }) return@transformWhile false
      if (result.await()) emit(Unit)
      previous = current
      true
    }.collect { emit(it) }
  }
  private fun <T> withVolumeRead(source: Flow<T>): Flow<T> = kotlinx.coroutines.flow.callbackFlow {
    val id = id()
    this@AudioLink.send(AudioInput.VolumeReadStarted(id))
    val observer = launch { try { source.collect { send(it) } } finally { channel.close() } }
    val close = onClose { channel.close() }
    awaitClose {
      observer.cancel()
      close.close()
      this@AudioLink.send(AudioInput.VolumeReadEnded(id))
    }
  }
  override fun close() {
    val finish: () -> Unit = {
      audioScope.launch {
      try {
        pending.values.forEach { it.completeExceptionally(MediaException.Closed()) }
        pending.clear()
        inputRegistry.values.forEach { it.stopLive() }
        inputRegistry.clear()
        tickets.values.toList().map { ticket ->
          async(start = CoroutineStart.UNDISPATCHED) { ticket.revoke(RecordingRefusal.INTERRUPTED) }
        }.awaitAll()
        runCatching { ports.focus.abandon() }.onFailure(::reportFocusFailure)
        if (communication) {
          try { ports.focus.leaveCommunication() } catch (failure: Exception) { Media.recordNativeFailure(FailureSource.SPEAKER, failure) } finally { communication = false }
        }
        runCatching { (ports.focus as? AutoCloseable)?.close() }.onFailure(Media::recordFailure)
        runCatching { (ports.permission as? AutoCloseable)?.close() }.onFailure(Media::recordFailure)
        calls.values.toList().forEach { runCatching { it.update(CallState.ENDED) }.onFailure(Media::recordFailure) }
        calls.clear()
        synchronized(closeListeners) { closeCompleted = true; closeListeners.toList().also { closeListeners.clear() } }.forEach { runCatching(it).onFailure(Media::recordFailure) }
      } catch (failure: Throwable) { Media.recordFailure(failure) } finally { closeDone.complete(Unit); audioScope.cancel(); thread.quitSafely() }
      }
    }
    val onThread = Looper.myLooper() == thread.looper
    synchronized(postLock) {
      if (!closed.compareAndSet(false, true)) return
      if (!onThread && !handler.post(finish)) { thread.quitSafely(); return }
    }
    if (onThread) finish()
  }
}

package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.FocusAnswer
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.PlayerEventsInterface
import com.noma4i.medialayer.PlayerSlot
import com.noma4i.medialayer.PlayerSlotFactory
import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import com.noma4i.medialayer.SlotSample
import com.noma4i.medialayer.testing.TestFetcher
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import org.robolectric.Shadows.shadowOf

/** The system focus, communication and speaker of Android. A test answers each request and fails chosen operations. */
internal class AppFocusPort : AudioFocusPort {
  val calls = SignallingList<String>()
  @Volatile var answer = FocusAnswer.GRANTED
  @Volatile var requestFailure: Exception? = null
  @Volatile var communicationFailure: Exception? = null
  private var sink: (FocusSignal) -> Unit = {}

  override fun listen(sink: (FocusSignal) -> Unit) { this.sink = sink }
  fun send(signal: FocusSignal) = sink(signal)

  override fun request(plan: FocusPlan): FocusAnswer {
    calls.add("request ${plan.gain} ${plan.usage}")
    requestFailure?.let { throw it }
    return answer
  }

  override fun abandon() { calls.add("abandon") }

  override fun enterCommunication(speaker: Boolean) {
    calls.add("communication speaker=$speaker")
    communicationFailure?.let { throw it }
  }

  override fun leaveCommunication() { calls.add("communication off") }
}

/** The microphone permission of the system: the app grants or revokes it while the app stays in the foreground. */
internal class TogglePermission(@Volatile var value: Boolean = true) : MicrophonePermissionPort {
  @Volatile private var sink: (com.noma4i.medialayer.MicrophonePermission) -> Unit = {}
  @Volatile private var override: com.noma4i.medialayer.MicrophonePermission? = null
  override fun state() = override ?: if (value) com.noma4i.medialayer.MicrophonePermission.GRANTED else com.noma4i.medialayer.MicrophonePermission.DENIED
  override fun listen(sink: (com.noma4i.medialayer.MicrophonePermission) -> Unit) { this.sink = sink }
  fun change(to: Boolean) { override = null; value = to; sink(state()) }
  fun change(to: com.noma4i.medialayer.MicrophonePermission) { override = to; value = to == com.noma4i.medialayer.MicrophonePermission.GRANTED; sink(to) }
  /** The system changed the permission and the AppOps callback has not come yet. */
  fun changeSilently(to: Boolean) { override = null; value = to }
}

/** The music stream volume of the system. */
internal class SteppedVolume(@Volatile var value: Float = 0.5f) : VolumePort {
  private val watchers = CopyOnWriteArrayList<() -> Unit>()
  val observers: Int get() = watchers.size
  override fun level(): Float? = value
  override fun observe(onChange: () -> Unit): () -> Unit {
    watchers.add(onChange)
    TestSignal.fire()
    return { watchers.remove(onChange); TestSignal.fire() }
  }
  fun set(level: Float) { value = level; watchers.forEach { it() } }
}

/** Native players of the app: each one reports ready on load and sounding on play, as ExoPlayer does. */
internal class PlaySlots : PlayerSlotFactory {
  private data class Event(val slot: UInt, val name: String, val key: String)
  private val events = CopyOnWriteArrayList<Event>()
  @Volatile var reports: PlayerEventsInterface? = null

  fun count(name: String) = events.count { it.name == name }

  fun playing(key: String): Boolean = replay(key) { it.second }
  fun muted(key: String): Boolean = replay(key) { it.first }

  private fun replay(key: String, read: (Pair<Boolean, Boolean>) -> Boolean): Boolean {
    val loaded = mutableMapOf<UInt, String>()
    val state = mutableMapOf<UInt, Pair<Boolean, Boolean>>()
    for (event in events) {
      val (muted, playing) = state[event.slot] ?: (false to false)
      when (event.name) {
        "load" -> { loaded[event.slot] = event.key; state[event.slot] = muted to false }
        "play" -> state[event.slot] = muted to true
        "pause", "unload" -> state[event.slot] = muted to false
        "mute" -> state[event.slot] = true to playing
        "unmute" -> state[event.slot] = false to playing
      }
    }
    return state.any { (slot, value) -> loaded[slot] == key && read(value) }
  }

  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(Player(slot))

  private inner class Player(private val slot: UInt) : PlayerSlot {
    @Volatile private var key = ""
    private val audible = AtomicBoolean()
    private fun note(name: String) { events.add(Event(slot, name, key)); TestSignal.fire() }
    private fun sounding(on: Boolean) { if (audible.compareAndSet(!on, on)) Media.link?.audible(slot.toULong(), on) }
    override suspend fun apply(command: SlotCall) {
      when (command) {
        is SlotCall.Load -> { key = command.key; note("load"); reports?.ready(slot, command.generation) }
        SlotCall.Unload -> { note("unload"); sounding(false) }
        SlotCall.Play -> { note("play"); sounding(true) }
        SlotCall.Pause -> { note("pause"); sounding(false) }
        SlotCall.Mute -> note("mute")
        SlotCall.Unmute -> note("unmute")
        is SlotCall.Rate -> note("rate")
        else -> {}
      }
    }
    override suspend fun dispose() {}
    override fun sample() = SlotSample(0u, null, false, false, false)
  }
}

/** A layer opened the way the app opens it: only the system ports are fakes. */
internal class AppPath(
  val focus: AppFocusPort = AppFocusPort(),
  val permission: TogglePermission = TogglePermission(),
  val volume: SteppedVolume = SteppedVolume(),
  val slots: PlaySlots = PlaySlots(),
  audioFocus: MediaAudioFocus = MediaAudioFocus.Exclusive,
  ports: AudioPorts = AudioPorts(focus, permission)
) : AutoCloseable {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val owner = object : LifecycleOwner {
    val registry: LifecycleRegistry by lazy { LifecycleRegistry.createUnsafe(this) }
    override val lifecycle: Lifecycle get() = registry
  }
  private val closed = AtomicBoolean()

  init {
    val manager = context.getSystemService(android.app.ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(android.app.ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
  }

  val layer: MediaLayer = openMediaLayer(
    context, TestFetcher(), SharedPreferencesSettings(context, "app-path-${System.nanoTime()}"),
    emulator = false, lifecycle = owner.registry, audioFocus = audioFocus,
    audioPorts = ports, volume = volume, slotFactory = slots
  ) { MediaLayer.open(it) }.blocking()

  init {
    slots.reports = layer.playerEvents()
    foreground(true)
  }

  fun foreground(on: Boolean) {
    owner.registry.currentState = if (on) Lifecycle.State.RESUMED else Lifecycle.State.CREATED
    shadowOf(Looper.getMainLooper()).idle()
  }

  override fun close() { if (closed.compareAndSet(false, true)) layer.close() }

  /** Closes the layer and returns after its shutdown, running the main thread while it waits. */
  fun closeAndAwait(timeoutMs: Long = 5_000) {
    close()
    val done = CoroutineScope(Dispatchers.Default).async { layer.awaitClosed() }
    done.invokeOnCompletion { TestSignal.fire() }
    TestSignal.await("the layer closed", timeoutMs) { shadowOf(Looper.getMainLooper()).idle(); done.isCompleted }
  }
}

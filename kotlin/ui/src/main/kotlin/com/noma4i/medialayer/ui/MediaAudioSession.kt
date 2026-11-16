package com.noma4i.medialayer.ui

import android.content.Context
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow

class MediaRecordingHold internal constructor(private val release: () -> Unit) : AutoCloseable {
  private val closed = AtomicBoolean()
  override fun close() { if (closed.compareAndSet(false, true)) release() }
}

class MediaAudioSession internal constructor(
  val focus: MediaAudioFocus,
  port: AudioFocusPort,
  releaseDelayMs: Long = RELEASE_DELAY_MS,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val volume: VolumePort? = null
) {
  private val demand = AudioDemand()
  private val recordingLock = Any()
  private val owner = AudioFocusOwner(port, { synchronized(demand) { demand.target(focus) } }, releaseDelayMs, scope)

  init { port.onChange { owner.reconcile() } }

  constructor(context: Context, focus: MediaAudioFocus) : this(focus, SystemAudioFocus(context), volume = SystemVolume(context))

  val isRecording: Boolean get() = synchronized(demand) { demand.recording }

  fun holdRecording(): MediaRecordingHold? = synchronized(recordingLock) {
    val id = UUID.randomUUID()
    val holders = synchronized(demand) { demand.setRecording(id, true); demand.recordings.size }
    MediaTrace.emit("audio") { "recording hold holders=$holders" }
    val hold = MediaRecordingHold {
      val count = synchronized(demand) { demand.setRecording(id, false); demand.recordings.size }
      MediaTrace.emit("audio") { "recording release holders=$count" }
      owner.reconcile()
    }
    if (owner.reconcile()) return@synchronized hold
    hold.close()
    null
  }

  fun readVolume(): MediaVolumeReading? = volume?.reading()

  fun volumeRaises(): Flow<Unit> = volume?.raises() ?: emptyFlow()

  internal fun setAudible(slot: UInt, value: Boolean) {
    if (synchronized(demand) { demand.setAudible(slot, value) }) MediaTrace.emit("audio") { "audible slot=$slot $value" }
    owner.reconcile()
  }

  internal fun close() {
    owner.close()
    scope.cancel()
  }

  private companion object {
    const val RELEASE_DELAY_MS = 300L
  }
}

package com.noma4i.medialayer.ui

internal class CameraAudioHold(
  private val link: () -> AudioLink?,
  private val tap: CameraAudioTap,
  private val issue: (CaptureAudioIssue) -> Unit,
  private val timer: RetryTimer = RetryTimer.system,
  private val microphone: (RecordingAudioListener) -> CameraMicrophone
) : AutoCloseable {
  private var input: CameraAudioInput? = null
  private var spec: PcmSpec? = null
  private var closed = false
  private val made = ArrayList<CameraMicrophone>()

  /** Returns once every microphone this hold opened has no live AudioRecord. */
  suspend fun idle() { synchronized(made) { made.toList() }.forEach { it.idle() } }

  val generation: Long? get() = input?.audioGeneration

  private var waiting: (() -> Unit)? = null

  /**
   * Runs [run] once the microphone is connected, at once when it is. The core hands the microphone to the camera only after the recording before it ended
   * (a voice stop): a take that starts in that gap waits for it instead of finding none. Returns the handle that drops the wait.
   */
  fun whenConnected(run: () -> Unit): AutoCloseable {
    synchronized(made) { waiting = run }
    // The generation is read outside the lock: the input tells `connected` under its own lock.
    if (generation != null) connected()
    return AutoCloseable { synchronized(made) { if (waiting === run) waiting = null } }
  }

  private fun connected() { synchronized(made) { waiting.also { waiting = null } }?.invoke() }

  fun keep(wanted: PcmSpec?) {
    if (closed && wanted == null) return
    check(!closed) { "the audio hold is closed" }
    if (wanted == spec) return
    input?.close()
    input = null
    spec = null
    if (wanted == null) return
    val audio = checkNotNull(link()) { "no audio link for the camera microphone" }
    input = CameraAudioInput(audio, wanted, tap, issue, timer = timer, connected = ::connected) { heard -> microphone(heard).also { synchronized(made) { made.add(it) } } }
    spec = wanted
  }

  /** Runs the microphone attempts again when the last run ended silent. */
  fun retry() { input?.retry() }

  override fun close() {
    if (closed) return
    closed = true
    input?.close()
    input = null
    spec = null
  }
}

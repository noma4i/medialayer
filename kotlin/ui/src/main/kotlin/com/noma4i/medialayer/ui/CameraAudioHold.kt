package com.noma4i.medialayer.ui

internal class CameraAudioHold(
  private val link: () -> AudioLink?,
  private val tap: CameraAudioTap,
  private val issue: (CaptureAudioIssue) -> Unit,
  private val microphone: (RecordingAudioListener) -> CameraMicrophone
) : AutoCloseable {
  private var input: CameraAudioInput? = null
  private var spec: PcmSpec? = null
  private var closed = false

  val generation: Long? get() = input?.audioGeneration

  fun keep(wanted: PcmSpec?) {
    if (closed && wanted == null) return
    check(!closed) { "the audio hold is closed" }
    if (wanted == spec) return
    input?.close()
    input = null
    spec = null
    if (wanted == null) return
    val audio = checkNotNull(link()) { "no audio link for the camera microphone" }
    input = CameraAudioInput(audio, wanted, tap, issue, microphone)
    spec = wanted
  }

  override fun close() {
    if (closed) return
    closed = true
    input?.close()
    input = null
    spec = null
  }
}

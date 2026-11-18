package com.noma4i.medialayer.testing

import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophonePermission
import com.noma4i.medialayer.ui.AudioDriveSignal
import com.noma4i.medialayer.ui.MediaLayerTestingApi
import com.noma4i.medialayer.ui.driveAudio

/**
 * Feeds system audio events into a real opened `MediaLayer` through the same inputs the system signals use,
 * so `VoiceState`, `CameraState` and `microphoneAvailability` run their real code without a real call.
 */
@OptIn(MediaLayerTestingApi::class)
class MediaAudioDriver(private val media: MediaLayer) {
  /** The microphone stops delivering sound: live takes end with `microphoneSilent`, availability becomes `NO_SOUND`. */
  fun silenceMicrophone() = media.driveAudio(AudioDriveSignal.SilenceMicrophone)
  fun restoreMicrophone() = media.driveAudio(AudioDriveSignal.RestoreMicrophone)
  /** A call of another app starts: takes end with `inCall`, availability becomes `EXTERNAL_CALL`. */
  fun startCall() = media.driveAudio(AudioDriveSignal.StartCall)
  fun endCall() = media.driveAudio(AudioDriveSignal.EndCall)
  fun beginInterruption() = media.driveAudio(AudioDriveSignal.BeginInterruption)
  /** Android has no resume hint: the audio focus returns. */
  fun endInterruption() = media.driveAudio(AudioDriveSignal.EndInterruption)
  /** Changes only the library state, not the system permission. */
  fun revokePermission() = media.driveAudio(AudioDriveSignal.Permission(MicrophonePermission.DENIED))
  /** Changes only the library state, not the system permission. */
  fun grantPermission() = media.driveAudio(AudioDriveSignal.Permission(MicrophonePermission.GRANTED))
}

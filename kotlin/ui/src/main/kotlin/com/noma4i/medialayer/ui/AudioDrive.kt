package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophonePermission

/** Marks the entry that feeds test signals into the audio link. Only `:testing` opts in. */
@RequiresOptIn(level = RequiresOptIn.Level.ERROR, message = "Only the MediaLayer testing module may drive the audio link")
@Retention(AnnotationRetention.BINARY)
annotation class MediaLayerTestingApi

@MediaLayerTestingApi
sealed interface AudioDriveSignal {
  data object SilenceMicrophone : AudioDriveSignal
  data object RestoreMicrophone : AudioDriveSignal
  data object StartCall : AudioDriveSignal
  data object EndCall : AudioDriveSignal
  data object BeginInterruption : AudioDriveSignal
  data object EndInterruption : AudioDriveSignal
  data class Permission(val state: MicrophonePermission) : AudioDriveSignal
}

/** Calls the same inputs the system ports call, so the core and the wrappers run their real paths. */
@MediaLayerTestingApi
fun MediaLayer.driveAudio(signal: AudioDriveSignal) {
  (binding?.link ?: throw com.noma4i.medialayer.MediaException.Closed()).drive(signal)
}

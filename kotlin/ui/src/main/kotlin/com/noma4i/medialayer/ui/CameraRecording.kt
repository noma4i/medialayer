package com.noma4i.medialayer.ui

import android.content.Context
import androidx.camera.video.AudioStats
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoRecordEvent
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordEnd
import java.io.File
import java.util.concurrent.Executor

internal class CameraRecording(
  private val context: Context,
  private val main: Executor,
  private val emit: (CaptureEvent) -> Unit,
  private val audioIssue: (CaptureAudioIssue) -> Unit
) {
  private var recording: Recording? = null
  private val stopCallbacks = mutableListOf<() -> Unit>()
  @Volatile private var recordedNanos = 0L

  val active: Boolean get() = recording != null

  fun recordedMs(): Long = recordedNanos / 1_000_000

  fun start(output: Recorder, file: File, profile: com.noma4i.medialayer.CaptureProfile, stale: () -> Boolean, finalized: () -> Unit) {
    try {
      recordedNanos = 0
      var pending = output.prepareRecording(context, FileOutputOptions.Builder(file).setDurationLimitMillis(profile.maxDurationMs.toLong()).build())
      if (profile.audio) pending = pending.withAudioEnabled()
      MediaTrace.emit("camera") { "take audio=${profile.audio}" }
      recording = pending.start(main) callback@{ event ->
        if (stale()) {
          if (event is VideoRecordEvent.Finalize) completeStops()
          return@callback
        }
        if (profile.audio) {
          when (event.recordingStats.audioStats.audioState) {
            AudioStats.AUDIO_STATE_SOURCE_SILENCED -> audioIssue(CaptureAudioIssue.SILENCED)
            AudioStats.AUDIO_STATE_ACTIVE -> audioIssue(CaptureAudioIssue.ACTIVE)
            AudioStats.AUDIO_STATE_SOURCE_ERROR -> audioIssue(CaptureAudioIssue.SOURCE_ERROR)
            AudioStats.AUDIO_STATE_ENCODER_ERROR -> audioIssue(CaptureAudioIssue.ENCODER_ERROR)
          }
        }
        when (event) {
          is VideoRecordEvent.Start -> emit(CaptureEvent.RecordStarted)
          is VideoRecordEvent.Status -> recordedNanos = event.recordingStats.recordedDurationNanos
          is VideoRecordEvent.Finalize -> {
            recording = null
            if (endOf(event.error) == RecordEnd.FAILED) {
              Media.recordNativeFailure(NativeFailure(FailureSource.CAMERA, "androidx.camera.video.VideoRecordEvent.Finalize", event.error.toLong(), event.cause?.message ?: "Recording failed"))
            }
            emit(CaptureEvent.RecordFinished(endOf(event.error)))
            completeStops()
            finalized()
          }
          else -> Unit
        }
      }
    } catch (error: SecurityException) {
      Media.recordNativeFailure(FailureSource.CAMERA, error)
      audioIssue(CaptureAudioIssue.PERMISSION_DENIED)
      emit(CaptureEvent.RecordFinished(RecordEnd.FAILED))
    } catch (error: Exception) {
      Media.recordNativeFailure(FailureSource.CAMERA, error)
      emit(CaptureEvent.RecordFinished(RecordEnd.FAILED))
    }
  }

  fun stop() { recording?.stop() }

  fun stop(done: () -> Unit) {
    val current = recording
    if (current == null) done() else {
      stopCallbacks.add(done)
      try { current.stop() } catch (error: Exception) {
        Media.recordNativeFailure(FailureSource.CAMERA, error)
        completeStops()
      }
    }
  }

  fun awaitStopped(done: () -> Unit) { if (recording == null) done() else stopCallbacks.add(done) }

  fun close() {
    recording?.close()
    recording = null
  }

  private fun completeStops() {
    val callbacks = stopCallbacks.toList()
    stopCallbacks.clear()
    callbacks.forEach { it() }
  }

  private fun endOf(error: Int) = when (error) {
    VideoRecordEvent.Finalize.ERROR_NONE -> RecordEnd.COMPLETED
    VideoRecordEvent.Finalize.ERROR_DURATION_LIMIT_REACHED -> RecordEnd.LIMIT_REACHED
    else -> RecordEnd.FAILED
  }
}

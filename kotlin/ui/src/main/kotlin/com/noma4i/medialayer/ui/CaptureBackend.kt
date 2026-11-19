package com.noma4i.medialayer.ui

import android.content.Context
import androidx.camera.core.ImageAnalysis
import androidx.camera.view.PreviewView
import androidx.lifecycle.LifecycleOwner
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureFileKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.captureFileName
import com.noma4i.medialayer.isCaptureOrphan
import java.io.File
import java.util.UUID

internal fun deleteMediaFile(file: File): Boolean = try {
  if (!file.exists() || file.delete()) true else {
    Media.recordNativeFailure(com.noma4i.medialayer.NativeFailure(com.noma4i.medialayer.FailureSource.FILE, "java.io.File", 0L, "Failed to delete ${file.path}"))
    false
  }
} catch (error: Exception) {
  Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.FILE, error)
  false
}

internal enum class CaptureAudioIssue { PERMISSION_DENIED, SOURCE_ERROR, ENCODER_ERROR, SILENCED, ACTIVE }

/**
 * The hardware side of a camera. Every method returns at once: the answer of the hardware comes back as an event to the sink.
 * `bind` ends with `Opened` and `Streaming`, or with `Refuse`. A finished or failed recording ends with `RecordFinished`.
 */
internal interface CaptureBackend {
  fun close() = Unit
  val photoSize: PixelSize?

  fun setSink(sink: (CaptureEvent) -> Unit)

  fun setAudioIssueSink(sink: (CaptureAudioIssue) -> Unit) = Unit

  /** The verdict of the sound a finished take wrote, judged at its write point: it arrives before the `RecordFinished` of the same take. */
  fun setCoverageSink(sink: (com.noma4i.medialayer.AudioCoverage) -> Unit) = Unit

  fun setHost(view: PreviewView?, owner: LifecycleOwner?)

  fun useMemory(memory: CaptureMemory) = Unit

  fun bind(lens: CameraLens, profile: CaptureProfile)

  fun unbind()

  /** Calls back once the audio input, the microphone and the take of the last [unbind] are closed: the memory of the session closes after it. */
  fun unbound(done: () -> Unit) = done()

  /** Writes the JPEG of a photo into [file]. */
  suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile)

  fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens)
  fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens, start: (() -> Unit) -> Unit) {
    start { beginRecording(file, profile, lens) }
  }

  fun endRecording()

  fun endRecording(done: () -> Unit) { try { endRecording() } finally { done() } }

  fun awaitRecordingStopped(done: () -> Unit) = done()

  /** Returns once the camera's microphones hold no AudioRecord: the recording ticket ends after it. */
  suspend fun audioIdle() = Unit

  /** The audio source of the take is gone. */
  fun audioLost() = Unit

  /** A start of a take with sound runs the microphone attempts again when its last run ended silent (owner 2026-11-19, [MicrophoneRetry]). */
  fun retryMicrophone() = Unit

  fun recordedMs(): Long

  fun setTorch(on: Boolean)

  fun setTorch(on: Boolean, done: (Throwable?) -> Unit) {
    try { setTorch(on); done(null) } catch (error: Exception) { done(error) }
  }

  fun setZoom(ratio: Float)

  fun setZoom(ratio: Float, done: (Throwable?) -> Unit) {
    try { setZoom(ratio); done(null) } catch (error: Exception) { done(error) }
  }

  fun setFrameTap(analyzer: ImageAnalysis.Analyzer?)

  fun sensorToView(): android.graphics.Matrix?
}

/** What a camera takes from its surroundings, so a test can give it a fake of each. */
internal class CameraServices(
  val admitSession: suspend (Long) -> AutoCloseable,
  val admitShot: suspend (Long) -> AutoCloseable,
  val audio: () -> AudioLink?,
  val clipInfo: suspend (File) -> MediaInfo?,
  /** True when it rewrote the file. */
  val uprightVideo: suspend (File) -> Boolean,
  val upright: suspend (File, Double) -> File,
  val directory: File,
  val audioCoverage: suspend (File) -> com.noma4i.medialayer.AudioCoverage
) {
  companion object {
    fun live(context: Context): CameraServices {
      val directory = CaptureFiles.directory(context)
      val environment = { ProcessMediaEnvironment.current ?: throw com.noma4i.medialayer.MediaException.Closed() }
      return CameraServices(
        admitSession = { bytes ->
          val hold = environment().media.reserveCapture(bytes.toULong())
          AutoCloseable { hold.release() }
        },
        admitShot = { bytes -> environment().media.decodeAdmit()(bytes) },
        audio = { Media.link },
        clipInfo = { file -> readMediaInfo(context, "file://${file.absolutePath}") },
        uprightVideo = { file -> uprightVideoAt(context, file) },
        upright = { raw, quality -> prepareImageAt(directory, raw, quality, environment().media.decodeAdmit()).file },
        directory = directory,
        audioCoverage = { file -> scanAudioCoverage(file) }
      )
    }
  }
}

/** The capture directory in the cache: the files of captures that the app has not taken yet. */
internal object CaptureFiles {
  fun directory(context: Context): File = File(context.cacheDir, "capture")

  fun name(kind: CaptureFileKind, directory: File): File {
    if (!directory.isDirectory && !directory.mkdirs() && !directory.isDirectory) throw java.io.IOException("Cannot create capture directory: $directory")
    return File(directory, captureFileName(kind, UUID.randomUUID().toString()))
  }

  /** Removes the files a capture of an earlier run left behind. Runs when the layer opens, when no camera of this run holds a file yet. */
  fun sweepOrphans(directory: File) {
    directory.listFiles()?.filter { isCaptureOrphan(it.name) }?.forEach { deleteMediaFile(it) }
  }
}

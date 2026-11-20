package com.noma4i.medialayer.ui

import android.content.Context
import androidx.camera.core.ImageAnalysis
import androidx.camera.view.PreviewView
import androidx.lifecycle.LifecycleOwner
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureFileKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.ImageFormat
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

  fun setHost(view: PreviewView?, owner: LifecycleOwner?)

  fun bind(lens: CameraLens, profile: CaptureProfile)

  fun unbind()

  /** Writes the JPEG of a photo into [file]. */
  suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile)

  fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens)
  fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens, start: (() -> Unit) -> Unit) {
    start { beginRecording(file, profile, lens) }
  }

  fun endRecording()

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
  val upright: suspend (File, Double) -> File,
  val directory: File,
  /** Makes the finished take upright in place; true when the file was replaced. A failure throws. */
  val uprightTake: suspend (File) -> Boolean = { false }
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
        clipInfo = { file -> runCatching { readMediaInfo(context, "file://${file.absolutePath}") }.getOrNull() },
        upright = { raw, quality ->
          val jpeg = encodeBytes(raw.readBytes(), ImageFormat.JPEG, quality, environment().media.decodeAdmit())
          File(directory, "prepared-${UUID.randomUUID()}.jpg").also { output ->
            try {
              output.writeBytes(jpeg)
            } catch (error: Throwable) {
              deleteMediaFile(output)
              throw error
            }
          }
        },
        directory = directory,
        uprightTake = { file -> makeTakeUpright(context, file) }
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

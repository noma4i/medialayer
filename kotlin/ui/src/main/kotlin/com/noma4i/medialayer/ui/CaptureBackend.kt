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

/**
 * The hardware side of a camera. Every method returns at once: the answer of the hardware comes back as an event to the sink.
 * `bind` ends with `Opened` and `Streaming`, or with `Refuse`. A finished or failed recording ends with `RecordFinished`.
 */
internal interface CaptureBackend {
  val photoSize: PixelSize?

  fun setSink(sink: (CaptureEvent) -> Unit)

  fun setHost(view: PreviewView?, owner: LifecycleOwner?)

  fun bind(lens: CameraLens, profile: CaptureProfile)

  fun unbind()

  /** Writes the JPEG of a photo into [file]. */
  suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile)

  fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens)

  fun endRecording()

  fun recordedMs(): Long

  fun setTorch(on: Boolean)

  fun setZoom(ratio: Float)

  fun setFrameTap(analyzer: ImageAnalysis.Analyzer?)

  fun sensorToView(): android.graphics.Matrix?
}

/** What a camera takes from its surroundings, so a test can give it a fake of each. */
internal class CameraServices(
  val admitSession: suspend (Long) -> AutoCloseable,
  val admitShot: suspend (Long) -> AutoCloseable,
  val audio: () -> MediaAudioSession?,
  val clipInfo: suspend (File) -> MediaInfo?,
  val upright: suspend (File, Double) -> File,
  val directory: File
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
        audio = { Media.audio },
        clipInfo = { file -> runCatching { readMediaInfo(context, "file://${file.absolutePath}") }.getOrNull() },
        upright = { raw, quality -> prepareImageAt(directory, raw, quality, environment().media.decodeAdmit()).file },
        directory = directory
      )
    }
  }
}

/** The capture directory in the cache: the files of captures that the app has not taken yet. */
internal object CaptureFiles {
  fun directory(context: Context): File = File(context.cacheDir, "capture")

  fun name(kind: CaptureFileKind, directory: File): File {
    directory.mkdirs()
    return File(directory, captureFileName(kind, UUID.randomUUID().toString()))
  }

  /** Removes the files a capture of an earlier run left behind. Runs when the layer opens, when no camera of this run holds a file yet. */
  fun sweepOrphans(directory: File) {
    directory.listFiles()?.filter { isCaptureOrphan(it.name) }?.forEach { it.delete() }
  }
}

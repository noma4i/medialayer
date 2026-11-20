package com.noma4i.medialayer.ui

import android.content.Context
import androidx.media3.common.Format
import com.noma4i.medialayer.captureUprightPlan
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.UUID
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/**
 * Makes the finished take upright: pixels as the viewer sees them and no turn in the matrix. The core decides from what the file holds ([captureUprightPlan]);
 * CameraX mirrors the front lens in pixels already, so `mirror` is false. A file that already is upright is not touched.
 * The turn is baked by the same Media3 export the prepare uses: it decodes the frames turned by the matrix and encodes them with portrait encoding, so no turn is written.
 * The new file replaces the take atomically; a failure leaves the take as it was and throws, and the caller fails the take. Returns whether the file was replaced.
 */
internal suspend fun makeTakeUpright(context: Context, file: File, exporter: suspend (VideoExport) -> Unit = ::exportPrepared): Boolean {
  val facts = readPrepareVideoFacts(file)
  val plan = captureUprightPlan(facts.width.toUInt(), facts.height.toUInt(), facts.rotation, false, (facts.bitrate ?: 0).toUInt(), facts.frameRate) ?: return false
  val started = System.nanoTime()
  val temp = File(file.parentFile, "${file.name}.${UUID.randomUUID()}.upright")
  try {
    // Codec, frame rate, sound and length are the take's: the audio goes through as it is, the frames only turn; the bitrate is the take's.
    val setup = TransformSetup(emptyList(), emptyList(), plan.bitrate.toInt().takeIf { it > 0 } ?: Format.NO_VALUE, Format.NO_VALUE, facts.mime, null)
    exporter(VideoExport(context, file, temp, setup, null))
    withContext(NonCancellable) { Files.move(temp.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING) }
  } finally {
    withContext(NonCancellable) { temp.delete() }
  }
  MediaTrace.emit("camera") { "take upright pass ms=${(System.nanoTime() - started) / 1_000_000} rotation=${plan.rotationDegrees}" }
  return true
}

package com.noma4i.medialayer.ui

import android.content.Context
import com.noma4i.medialayer.MediaPrepareFailure
import com.noma4i.medialayer.MediaPrepareJob
import com.noma4i.medialayer.MediaPrepareOutcome
import com.noma4i.medialayer.MediaProbeOutcome
import com.noma4i.medialayer.PrepareDecision
import com.noma4i.medialayer.PreparePlan
import com.noma4i.medialayer.PrepareProbe
import com.noma4i.medialayer.mediaPrepareDecide
import com.noma4i.medialayer.mediaPrepareFacts
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * Prepares a file for sending by a descriptor of `mediaPrepareDescriptor`: it probes the source, lets the core decide, then copies the source, writes the plan or refuses.
 * The result is written next to the destination and renamed, so a partial file never stays. `cancel` stops a run at once and the run returns Cancelled.
 */
class MediaPreparer internal constructor(
  private val context: Context,
  private val videoFacts: (File) -> VideoFacts,
  private val metadataKeys: (Context, File) -> List<String>,
  private val decoderList: (String) -> List<DecoderAbility>,
  private val exporter: suspend (VideoExport) -> Unit
) {
  constructor(context: Context) : this(context.applicationContext, ::readPrepareVideoFacts, ::readPrepareMetadataKeys, ::readDecoders, ::exportPrepared)

  private class Attempt(val job: Job)

  private val running = ConcurrentHashMap<ULong, Attempt>()

  /** The file facts of [path]. With a [descriptor] they are the planned result, without one the file as it is, found by [mime] or by reading it. */
  suspend fun probe(path: String, descriptor: ByteArray?, mime: String?): MediaProbeOutcome = withContext(prepareDispatcher) {
    try {
      val file = File(path)
      if (!file.isFile || !file.canRead()) return@withContext MediaProbeOutcome.Failed("$path is not a readable file")
      val kind = if (descriptor != null) kindOf(descriptor) else kindOfMime(mime)
      val probe = when (kind) {
        Kind.VIDEO -> readVideo(file)
        Kind.IMAGE -> probeOfImage(file) ?: PrepareProbe.Failed("$path is not an image")
        null -> if (descriptor != null) PrepareProbe.Failed("the prepare descriptor has an unknown version or is broken") else probeUnknown(file)
      }
      if (probe is PrepareProbe.Failed) return@withContext MediaProbeOutcome.Failed(probe.message)
      val facts = mediaPrepareFacts(descriptor, probe) ?: return@withContext MediaProbeOutcome.Failed("the file does not fit the prepare descriptor")
      MediaProbeOutcome.File(facts.width, facts.height, facts.mime)
    } catch (error: CancellationException) {
      throw error
    } catch (error: Exception) {
      MediaProbeOutcome.Failed(error.message ?: error.javaClass.simpleName)
    }
  }

  /** Prepares the source of [job] into its destination. [progress] gets 0 to 1, never going back. */
  suspend fun run(job: MediaPrepareJob, progress: (Double) -> Unit): MediaPrepareOutcome = coroutineScope {
    val work = async(prepareDispatcher, start = CoroutineStart.LAZY) { write(job, steady(progress)) }
    val attempt = Attempt(work)
    running[job.attemptId] = attempt
    try {
      work.await()
    } catch (cancelled: CancellationException) {
      currentCoroutineContext().ensureActive()
      MediaPrepareOutcome.Cancelled
    } finally {
      running.remove(job.attemptId, attempt)
    }
  }

  /** Stops the run of [attemptId] at once: its temp file is removed and the run returns Cancelled. */
  fun cancel(attemptId: ULong) {
    running[attemptId]?.job?.cancel()
  }

  private fun steady(report: (Double) -> Unit): (Double) -> Unit {
    var last = 0.0
    val lock = Any()
    return { value ->
      val next = synchronized(lock) { value.coerceIn(last, 1.0).also { last = it } }
      report(next)
    }
  }

  private enum class Kind { VIDEO, IMAGE }

  // The descriptor starts with the version 1 and the kind (0 video, 1 image), see core/src/send_prep/descriptor.rs.
  private fun kindOf(descriptor: ByteArray): Kind? =
    if (descriptor.size < 2 || descriptor[0] != 1.toByte()) null else when (descriptor[1].toInt()) { 0 -> Kind.VIDEO; 1 -> Kind.IMAGE; else -> null }

  private fun kindOfMime(mime: String?): Kind? = when {
    mime == null -> null
    mime.startsWith("video/") -> Kind.VIDEO
    mime.startsWith("image/") -> Kind.IMAGE
    else -> null
  }

  private fun readVideo(file: File): PrepareProbe = probeOfVideo(videoFacts(file), metadataKeys(context, file), file.length())

  private fun probeUnknown(file: File): PrepareProbe = probeOfImage(file) ?: readVideo(file)

  private suspend fun write(job: MediaPrepareJob, progress: (Double) -> Unit): MediaPrepareOutcome {
    val source = File(job.source)
    val destination = File(job.destination)
    val temp = File(destination.absoluteFile.parentFile, ".${destination.name}.${job.attemptId}.tmp")
    var outcome: MediaPrepareOutcome = MediaPrepareOutcome.Failed("the preparation did not finish")
    try {
      if (!source.isFile || !source.canRead()) return MediaPrepareOutcome.Failed("${job.source} is not a readable file").also { outcome = it }
      val kind = kindOf(job.descriptor)
      val probe = when (kind) {
        Kind.VIDEO -> readVideo(source)
        Kind.IMAGE -> probeOfImage(source) ?: PrepareProbe.Failed("${job.source} is not an image")
        null -> PrepareProbe.Failed("the prepare descriptor has an unknown version or is broken")
      }
      outcome = when (val decision = mediaPrepareDecide(job.descriptor, probe)) {
        is PrepareDecision.Refuse -> refused(decision.reason, decision.message)
        is PrepareDecision.Pass -> pass(source, temp, destination, mimeOfPass(job.descriptor, probe), progress)
        is PrepareDecision.Run -> when (val plan = decision.plan) {
          is PreparePlan.Video -> writeVideo(job, source, temp, destination, plan, progress)
          is PreparePlan.Image -> writeImage(source, temp, destination, plan, progress)
        }
      }
    } catch (error: CancellationException) {
      throw error
    } catch (error: Exception) {
      outcome = MediaPrepareOutcome.Failed(error.message ?: error.javaClass.simpleName)
    } finally {
      withContext(NonCancellable) { temp.delete() }
    }
    return outcome
  }

  private fun refused(reason: MediaPrepareFailure, message: String): MediaPrepareOutcome =
    if (reason == MediaPrepareFailure.CANNOT_DECODE) MediaPrepareOutcome.CannotDecode(message) else MediaPrepareOutcome.Failed(message)

  private fun mimeOfPass(descriptor: ByteArray, probe: PrepareProbe): String? = mediaPrepareFacts(descriptor, probe)?.mime

  private suspend fun pass(source: File, temp: File, destination: File, mime: String?, progress: (Double) -> Unit): MediaPrepareOutcome {
    source.copyTo(temp, overwrite = true)
    currentCoroutineContext().ensureActive()
    return finish(temp, destination, mime, progress)
  }

  private suspend fun writeVideo(job: MediaPrepareJob, source: File, temp: File, destination: File, plan: PreparePlan.Video, progress: (Double) -> Unit): MediaPrepareOutcome {
    val facts = videoFacts(source)
    val encode = plan.plan.reencode
    if (encode) {
      decoderRefusal(facts, decoderList(facts.mime))?.let {
        // The app may ask for the source as it is when the device cannot decode it: the last byte of a video descriptor is that policy.
        return if (job.descriptor.last() == 1.toByte()) pass(source, temp, destination, "video/mp4", progress) else MediaPrepareOutcome.CannotDecode(it)
      }
    }
    exporter(VideoExport(context, source, temp, transformSetup(plan, facts, encode)) { progress(it) })
    currentCoroutineContext().ensureActive()
    return finish(temp, destination, "video/mp4", progress)
  }

  private suspend fun writeImage(source: File, temp: File, destination: File, plan: PreparePlan.Image, progress: (Double) -> Unit): MediaPrepareOutcome {
    val written = writePreparedImage(source, temp, plan) ?: return MediaPrepareOutcome.Failed("${source.path} is not an image")
    return finish(temp, destination, written.first, progress)
  }

  private suspend fun finish(temp: File, destination: File, mime: String?, progress: (Double) -> Unit): MediaPrepareOutcome {
    currentCoroutineContext().ensureActive()
    Files.move(temp.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING)
    progress(1.0)
    return MediaPrepareOutcome.Done(null, mime)
  }
}

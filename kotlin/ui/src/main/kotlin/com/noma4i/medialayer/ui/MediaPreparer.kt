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

  /** The file facts of [path]. With a [descriptor] they are the planned result, without one the file as it is. [mime] tells a video from an image, without it the file is read to find out. */
  suspend fun probe(path: String, descriptor: ByteArray?, mime: String?): MediaProbeOutcome = withContext(prepareDispatcher) {
    try {
      val file = File(path)
      if (!file.isFile || !file.canRead()) MediaProbeOutcome.Failed("$path is not a readable file")
      else mediaPrepareFacts(descriptor, probeOf(file, mime))
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

  private fun probeOf(file: File, mime: String?): PrepareProbe = when {
    mime?.startsWith("video/") == true -> readVideo(file)
    mime?.startsWith("image/") == true -> probeOfImage(file) ?: PrepareProbe.Failed("${file.path} is not an image")
    else -> probeOfImage(file) ?: readVideo(file)
  }

  private fun readVideo(file: File): PrepareProbe = probeOfVideo(videoFacts(file), metadataKeys(context, file), file.length())

  private suspend fun write(job: MediaPrepareJob, progress: (Double) -> Unit): MediaPrepareOutcome {
    val source = File(job.source)
    val destination = File(job.destination)
    val temp = File(destination.absoluteFile.parentFile, ".${destination.name}.${job.attemptId}.tmp")
    var outcome: MediaPrepareOutcome = MediaPrepareOutcome.Failed("the preparation did not finish")
    try {
      if (!source.isFile || !source.canRead()) return MediaPrepareOutcome.Failed("${job.source} is not a readable file").also { outcome = it }
      val probe = try {
        probeOf(source, null)
      } catch (error: CancellationException) {
        throw error
      } catch (error: Exception) {
        PrepareProbe.Failed(error.message ?: error.javaClass.simpleName)
      }
      outcome = when (val decision = mediaPrepareDecide(job.descriptor, probe)) {
        is PrepareDecision.Refuse -> refused(decision.reason, decision.message)
        is PrepareDecision.Pass -> unchanged(job.descriptor, probe)
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

  /** The source goes as it is: nothing is copied and nothing is written. */
  private fun unchanged(descriptor: ByteArray, probe: PrepareProbe): MediaPrepareOutcome =
    MediaPrepareOutcome.Unchanged(null, (mediaPrepareFacts(descriptor, probe) as? MediaProbeOutcome.File)?.mime)

  private suspend fun writeVideo(job: MediaPrepareJob, source: File, temp: File, destination: File, plan: PreparePlan.Video, progress: (Double) -> Unit): MediaPrepareOutcome {
    val facts = videoFacts(source)
    val encode = plan.plan.reencode
    if (encode) {
      // The core owns the policy of a source the device cannot decode: it answers Pass or Refuse(CannotDecode) to a failed probe.
      decoderRefusal(facts, decoderList(facts.mime))?.let {
        return when (val decision = mediaPrepareDecide(job.descriptor, PrepareProbe.Failed(it))) {
          is PrepareDecision.Pass -> MediaPrepareOutcome.Unchanged(null, "video/mp4")
          is PrepareDecision.Refuse -> refused(decision.reason, decision.message)
          is PrepareDecision.Run -> MediaPrepareOutcome.Failed(it)
        }
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

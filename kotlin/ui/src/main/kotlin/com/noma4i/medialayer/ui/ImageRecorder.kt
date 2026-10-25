package com.noma4i.medialayer.ui

import com.noma4i.medialayer.ImageCounters
import com.noma4i.medialayer.ImageTotals
import com.noma4i.medialayer.ImageWork
import java.util.Collections
import java.util.WeakHashMap

private val recorders: MutableMap<ImageCounters, ImageRecorder> = Collections.synchronizedMap(WeakHashMap())

/** Coalesces image work records into totals; one drain applies them, so a record costs no task. */
internal class ImageRecorder(counters: ImageCounters) {
  private val counters = java.lang.ref.WeakReference(counters)
  private val lock = Any()
  private var requests = 0uL
  private var placementPixels = 0uL
  private var decodes = 0uL
  private var decodedPixels = 0uL
  private var bitmapBytes = 0uL
  private var finished = 0uL
  private var memoryHits = 0uL
  private var cancellations = 0uL
  private var diskHits = 0uL
  private var observed = 0L
  private val drain = LatestDrain<Boolean>({ _, _ -> true }) { flush() }

  fun record(work: ImageWork) {
    synchronized(lock) {
      when (work) {
        is ImageWork.Request -> { requests++; placementPixels += work.placementPixels }
        is ImageWork.Decode -> { decodes++; decodedPixels += work.width.toULong() * work.height.toULong(); bitmapBytes += work.bytes }
        is ImageWork.Finish -> { finished++; if (work.memoryHit) memoryHits++ }
        is ImageWork.Cancel -> cancellations++
        is ImageWork.DiskHit -> diskHits++
        is ImageWork.Observe -> observed++
        is ImageWork.Unobserve -> observed--
      }
    }
    drain.submit(true)
  }

  private fun flush() {
    val totals = synchronized(lock) {
      ImageTotals(requests, placementPixels, decodes, decodedPixels, bitmapBytes, finished, memoryHits, cancellations, diskHits, observed).also {
        requests = 0uL; placementPixels = 0uL; decodes = 0uL; decodedPixels = 0uL; bitmapBytes = 0uL
        finished = 0uL; memoryHits = 0uL; cancellations = 0uL; diskHits = 0uL; observed = 0L
      }
    }
    counters.get()?.add(totals)
  }
}

internal fun ImageCounters.recordLater(work: ImageWork) = synchronized(recorders) { recorders.getOrPut(this) { ImageRecorder(this) } }.record(work)

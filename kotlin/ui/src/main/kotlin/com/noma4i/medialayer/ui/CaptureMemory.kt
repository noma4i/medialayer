package com.noma4i.medialayer.ui

import java.util.concurrent.atomic.AtomicBoolean

/** The session of the memory ended: a lease asked after it is no device failure but the quiet end of its asker. */
internal class CaptureMemoryClosed : IllegalStateException("the capture memory is closed")

/** One admitted capture hold split into leases: the GL input, the microphone and the held take share [total]. `close(onDrained)` calls back once, outside the lock, after the last lease came back. */
internal class CaptureMemory(val total: Long) : AutoCloseable {
  private val lock = Any()
  private var leasedBytes = 0L
  private var closed = false
  private var drained: (() -> Unit)? = null

  val leased: Long get() = synchronized(lock) { leasedBytes }

  val available: Long get() = synchronized(lock) { total - leasedBytes }

  fun lease(bytes: Long): AutoCloseable = synchronized(lock) {
    if (closed) throw CaptureMemoryClosed()
    require(bytes > 0) { "a lease asks for no bytes" }
    check(bytes <= total - leasedBytes) { "a lease of $bytes bytes does not fit the ${total - leasedBytes} bytes left of $total" }
    leasedBytes += bytes
    val returned = AtomicBoolean()
    AutoCloseable {
      if (returned.compareAndSet(false, true)) {
        val done = synchronized(lock) {
          leasedBytes -= bytes
          if (closed && leasedBytes == 0L) drained.also { drained = null } else null
        }
        done?.invoke()
      }
    }
  }

  override fun close() = close {}

  fun close(onDrained: () -> Unit) {
    val now = synchronized(lock) {
      if (closed) return
      closed = true
      if (leasedBytes == 0L) true else { drained = onDrained; false }
    }
    if (now) onDrained()
  }
}

internal fun framesInFlight(total: Long, width: Int, height: Int): Int {
  require(width > 0 && height > 0) { "the frame has no size" }
  val frames = total / (width.toLong() * height * RGBA_BYTES)
  check(frames > 0) { "$total bytes do not hold one ${width}x$height frame" }
  return frames.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
}

private const val RGBA_BYTES = 4L

package com.noma4i.medialayer.ui

import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

/**
 * owner 2026-11-19: a start of the microphone (voice and camera) tries it up to [ATTEMPTS] times. Each attempt waits at most [SOUND_WAIT_MS] for the first live sound,
 * [PAUSE_MS] pass between attempts. An attempt fails when it gives no sound in time, its source does not start or the system silences it before its first PCM.
 * The app sees only the outcome: the first sound of any attempt, or the refusal of the last one.
 */
internal object MicrophoneRetry {
  const val ATTEMPTS = 3
  const val SOUND_WAIT_MS = 1_000L
  const val PAUSE_MS = 200L
}

/** The clock of the attempts: runs [run] once after [delayMs] unless the returned handle is closed first. */
internal fun interface RetryTimer {
  fun schedule(delayMs: Long, run: () -> Unit): AutoCloseable

  companion object {
    private val executor: ScheduledExecutorService by lazy {
      Executors.newSingleThreadScheduledExecutor { Thread(it, "MicrophoneRetry").apply { isDaemon = true } }
    }

    val system = RetryTimer { delayMs, run ->
      val future = executor.schedule(run, delayMs, TimeUnit.MILLISECONDS)
      AutoCloseable { future.cancel(false) }
    }
  }
}

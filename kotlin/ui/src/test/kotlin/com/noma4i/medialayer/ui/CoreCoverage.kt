package com.noma4i.medialayer.ui

import com.noma4i.medialayer.AudioCoverage
import com.noma4i.medialayer.AudioInterval
import com.noma4i.medialayer.CoverageConfig
import com.noma4i.medialayer.CoverageVerdict
import com.noma4i.medialayer.VideoBounds
import com.noma4i.medialayer.coverageBegin
import com.noma4i.medialayer.coverageFinish
import com.noma4i.medialayer.coveragePush

/** What the scan hands the camera in a test: the answer of the real core rule for audio laid out as the file of the shape would give it, never a verdict typed by hand. */
internal object CoreCoverage {
  private const val RATE = 44_100u
  private const val FRAME = 1024u
  private const val STEP_US = 1024L * 1_000_000L / 44_100L

  private fun run(track: Boolean, packets: List<Int>, video: VideoBounds?, rate: UInt = RATE): AudioCoverage {
    var scan = coverageBegin(CoverageConfig(true, track, rate, FRAME))
    packets.forEach { at -> scan = coveragePush(scan, AudioInterval(at * STEP_US, (at + 1) * STEP_US, FRAME.toULong())) }
    return coverageFinish(scan, video)
  }

  private val video = VideoBounds(0L, 10 * STEP_US)

  fun covered() = run(true, (0 until 10).toList(), video)
  fun missing() = run(false, emptyList(), video)
  fun empty() = run(true, emptyList(), video)
  fun broken() = run(true, listOf(0), video, rate = 0u)
  fun middle() = run(true, listOf(0, 1, 2, 3, 7, 8, 9), video)
  fun head() = run(true, (4 until 10).toList(), video)
  fun tail() = run(true, (0 until 5).toList(), video)

  val refused = mapOf(
    CoverageVerdict.MISSING to missing(), CoverageVerdict.EMPTY to empty(), CoverageVerdict.BROKEN to broken(),
    CoverageVerdict.MID_GAP to middle(), CoverageVerdict.HEAD_GAP to head(), CoverageVerdict.TAIL_CUT to tail()
  )
}

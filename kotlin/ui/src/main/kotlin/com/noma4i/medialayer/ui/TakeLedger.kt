package com.noma4i.medialayer.ui

import com.noma4i.medialayer.AudioCoverage
import com.noma4i.medialayer.AudioInterval
import com.noma4i.medialayer.CoverageConfig
import com.noma4i.medialayer.CoverageScan
import com.noma4i.medialayer.VideoBounds
import com.noma4i.medialayer.coverageBegin
import com.noma4i.medialayer.coverageFinish
import com.noma4i.medialayer.coveragePush

/** What a take wrote, kept at the one write point of its muxer: one interval per audio sample on the timeline of the take, the first and the end of its video. The verdict comes from the same core rule the file scan runs, so the file is not read back after the stop. */
internal class TakeLedger(config: CoverageConfig) {
  private var scan: CoverageScan = coverageBegin(config)
  private var videoStartUs = -1L
  private var videoEndUs = -1L

  /** One audio sample written at [startUs] holding [samples] PCM samples per channel. */
  fun audio(startUs: Long, endUs: Long, samples: Int) {
    scan = coveragePush(scan, AudioInterval(startUs, endUs, samples.toULong()))
  }

  /** The video written so far: the first PTS stays, the end is the end of the last written frame. */
  fun video(startUs: Long, endUs: Long) {
    if (videoStartUs < 0) videoStartUs = startUs
    videoEndUs = endUs
  }

  fun coverage(): AudioCoverage = coverageFinish(scan, if (videoStartUs < 0) null else VideoBounds(videoStartUs, videoEndUs))

  companion object {
    /** A take without sound in its profile is not required to hold any, the core then skips it. */
    fun of(audio: Boolean, rate: Int, frame: Int) = TakeLedger(CoverageConfig(audio, audio, rate.toUInt(), if (audio) frame.toUInt() else 0u))
  }
}

package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaState
import org.junit.Assert.assertEquals
import org.junit.Test

class VideoEdgesTest {
  @Test
  fun inv_mount_02_firstFrameFiresOncePerSourceAcrossLoadingAndEndFiresEveryTimeTheClipEnds() {
    val edges = VideoEdges()
    val steps = listOf(
      Triple(MediaState.Loading, false, false), Triple(MediaState.Ready, false, false), Triple(MediaState.Rendered, true, false),
      Triple(MediaState.Rendered, false, false), Triple(MediaState.Ended, false, true), Triple(MediaState.Rendered, false, false),
      Triple(MediaState.Ended, false, true), Triple(MediaState.Loading, false, false), Triple(MediaState.Rendered, false, false),
      Triple(MediaState.Idle, false, false), Triple(MediaState.Rendered, true, false)
    )

    for ((state, firstFrame, ended) in steps) assertEquals("$state", firstFrame to ended, edges.advance(state))
  }
}

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

  @Test
  fun inv_placeholder_10_theCoverWaitsForTheRenderedFrameAndAReturnOfTheSameSourceKeepsItShown() {
    val frame = ShownFrame()
    val url = "https://o.test/a.mp4"
    val steps = listOf(
      MediaState.Idle to false, MediaState.Loading to false, MediaState.Ready to false, MediaState.Failed("x") to false, MediaState.Rendered to true,
      MediaState.Idle to true, MediaState.Loading to true, MediaState.Ready to true, MediaState.Ended to true
    )

    for ((state, shown) in steps) assertEquals("$state", shown, frame.advance(url, state))
    assertEquals(false, frame.advance("https://o.test/b.mp4", MediaState.Ready))
    assertEquals(true, frame.advance("https://o.test/b.mp4", MediaState.Ended))
  }
}

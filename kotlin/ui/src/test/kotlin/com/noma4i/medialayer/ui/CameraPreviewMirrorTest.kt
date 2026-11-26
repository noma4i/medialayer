package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CameraLens
import org.junit.Assert.assertEquals
import org.junit.Test

class CameraPreviewMirrorTest {
  @Test fun counterFlipsOnlyUnmirroredFrontPreview() {
    val cases = listOf(
      Triple(CameraLens.FRONT, false, true),
      Triple(CameraLens.FRONT, true, false),
      Triple(CameraLens.BACK, false, false),
      Triple(CameraLens.BACK, true, false)
    )
    for ((lens, mirror, expected) in cases) {
      assertEquals(expected, previewCounterFlip(lens, mirror))
    }
  }

  @Test fun previewMirrorFollowsLensAndCounterFlip() {
    val cases = listOf(
      Triple(CameraLens.FRONT, false, true),
      Triple(CameraLens.FRONT, true, false),
      Triple(CameraLens.BACK, false, false),
      Triple(CameraLens.BACK, true, false)
    )
    for ((lens, flip, expected) in cases) {
      assertEquals(expected, previewMirrored(lens, flip))
    }
  }
}

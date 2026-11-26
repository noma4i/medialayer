package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CameraLens
import org.junit.Assert.assertEquals
import org.junit.Test

class CameraPreviewMirrorTest {
  @Test fun previewMirrorFollowsLensAndFlag() {
    val cases = listOf(
      Triple(CameraLens.FRONT, false, false),
      Triple(CameraLens.FRONT, true, true),
      Triple(CameraLens.BACK, false, false),
      Triple(CameraLens.BACK, true, false)
    )
    for ((lens, mirror, expected) in cases) {
      assertEquals(expected, previewMirrored(lens, mirror))
    }
  }
}

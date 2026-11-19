package com.noma4i.medialayer.ui

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

// INV-CAPTURE-80: the recorded pixels stand as the preview showed them, in any pose of the device.
// The only source of the turn is the sensor rotation the preview uses; no pose sensor (OrientationEventListener) takes part, so no value of it can turn a take.
@RunWith(MediaLayerTestRunner::class)
class CaptureUprightTest {
  /** Where the vertex matrix of a take sends the middle of an edge of the sensor frame: the marker pixel of the output. */
  private fun sent(edge: String, rotation: Int, mirror: Boolean): String {
    val point = when (edge) {
      "left" -> floatArrayOf(-1f, 0f, 0f, 1f)
      "right" -> floatArrayOf(1f, 0f, 0f, 1f)
      "top" -> floatArrayOf(0f, 1f, 0f, 1f)
      else -> floatArrayOf(0f, -1f, 0f, 1f)
    }
    val out = FloatArray(4)
    android.opengl.Matrix.multiplyMV(out, 0, uprightVertex(rotation, mirror), 0, point, 0)
    val x = Math.round(out[0])
    val y = Math.round(out[1])
    return if (x < 0) "left" else if (x > 0) "right" else if (y > 0) "top" else "bottom"
  }

  @Test
  fun inv_capture_80_the_sensor_edge_the_preview_shows_on_top_is_on_top_of_the_take_front_and_back_mirrored_or_not() {
    // The preview turns the sensor frame clockwise by the sensor rotation: 90 puts the left edge on top, 270 the right one, 0 the top, 180 the bottom.
    for ((rotation, edge) in listOf(0 to "top", 90 to "left", 180 to "bottom", 270 to "right")) for (mirror in listOf(false, true)) {
      assertEquals("rotation $rotation mirror $mirror", "top", sent(edge, rotation, mirror))
    }
  }
}

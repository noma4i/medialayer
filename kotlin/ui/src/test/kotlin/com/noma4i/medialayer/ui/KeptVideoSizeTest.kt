package com.noma4i.medialayer.ui

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.ContentScale
import org.junit.Assert.assertEquals
import org.junit.Test

class KeptVideoSizeTest {
  // mutation: let the frame follow the player size to zero when the slot leaves -> videoFrame() falls back to the
  // whole box, the kept Crop TextureView is resized and its buffer goes black for ~2 frames at swipe start
  // (Samsung A16, MediaLayer 3.30.3, feed-cover2 rec-1 swipes 2, 6, 8)
  @Test
  fun aVideoThatLeftItsPlayerKeepsTheFrameOfItsLastSize() {
    val kept = KeptVideoSize()
    val box = Size(1080f, 1920f)
    val landscape = Size(1920f, 1080f)

    assertEquals(Size.Zero, kept.advance(Size.Zero))
    assertEquals(landscape, kept.advance(landscape))
    assertEquals(landscape, kept.advance(Size.Zero))
    assertEquals(landscape, kept.advance(Size.Unspecified))
    assertEquals(videoFrame(box, landscape, ContentScale.Crop), videoFrame(box, kept.advance(Size.Zero), ContentScale.Crop))

    val portrait = Size(1080f, 1920f)
    assertEquals(portrait, kept.advance(portrait))
  }
}

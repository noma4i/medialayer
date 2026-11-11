package com.noma4i.medialayer.ui

import androidx.compose.ui.Alignment
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.times
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import org.junit.Assert.assertEquals
import org.junit.Test

class VideoFrameTest {
  private fun coverFrame(box: Size, media: Size, scale: ContentScale): Rect {
    val scaled = media * scale.computeScaleFactor(media, box)
    val offset = Alignment.Center.align(
      IntSize(Math.round(scaled.width), Math.round(scaled.height)),
      IntSize(Math.round(box.width), Math.round(box.height)),
      LayoutDirection.Ltr
    )
    return Rect(offset.x.toFloat(), offset.y.toFloat(), offset.x + scaled.width, offset.y + scaled.height)
  }

  private fun assertSameRect(expected: Rect, actual: Rect) {
    listOf(expected.left to actual.left, expected.top to actual.top, expected.right to actual.right, expected.bottom to actual.bottom)
      .forEach { (e, a) -> assertEquals("expected $expected, got $actual", e, a, 1f) }
  }

  // mutation: clamp Crop to the box for the video (letterbox) while the cover really crops -> the feed video
  // reframes at the cover -> video switch (Android, owner complaint 2026-11-12, feed-cover rec-1)
  @Test
  fun theVideoIsFramedLikeItsCoverForEveryScaleAndAspect() {
    val box = Size(1080f, 1920f)
    listOf(Size(1920f, 1080f), Size(1080f, 1920f), Size(720f, 720f), Size(1080f, 2400f)).forEach { media ->
      listOf(ContentScale.Crop, ContentScale.Fit).forEach { scale ->
        assertSameRect(coverFrame(box, media, scale), videoFrame(box, media, scale))
      }
    }
    assertSameRect(Rect(-1166.67f, 0f, 2246.67f, 1920f), videoFrame(box, Size(1920f, 1080f), ContentScale.Crop))
    assertSameRect(Rect(0f, 656.25f, 1080f, 1263.75f), videoFrame(box, Size(1920f, 1080f), ContentScale.Fit))
  }

  // mutation: frame an unknown media size as zero (the video disappears until its size arrives)
  @Test
  fun aVideoOfUnknownSizeFillsItsBox() {
    assertSameRect(Rect(0f, 0f, 1080f, 1920f), videoFrame(Size(1080f, 1920f), Size.Unspecified, ContentScale.Crop))
    assertSameRect(Rect(0f, 0f, 1080f, 1920f), videoFrame(Size(1080f, 1920f), Size.Zero, ContentScale.Crop))
  }
}

package com.noma4i.medialayer.ui

import java.util.Base64
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private val animatedWebp = Base64.getDecoder().decode("UklGRrQAAABXRUJQVlA4WAoAAAACAAAADwAADwAAQU5JTQYAAAAAAAAAAABBTk1GKAAAAAAAAAAAAAcAAAcAAGQAAANWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAIAAAAAAAcAAAcAAGQAAAFWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAQAAAAAAAcAAAcAAGQAAABWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQA=")
private val animatedGif = Base64.getDecoder().decode("R0lGODlhEAAQAIEAAAAAAP8AAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQJCgAAACwAAAAAEAAQAAAIMAADCBwoEIDBgwYJEkSIUOFAhgcdFoQIQGIAihUlYrS4USPGjyBDihxJsqTJkyADAgAh+QQJCgAAACwEAAAACAAIAIEAAAD/AAAAAAAAAAAIDwADCBxIsKDBgwgTKkwYEAAh+QQJCgAAACwIAAAACAAIAIEAAAD/AAAAAAAAAAAIDwADCBxIsKDBgwgTKkwYEAA7")

private class RecordingSurface : FrameSurface {
  val steps = mutableListOf<String>()
  override fun clear(x: Int, y: Int, width: Int, height: Int) { steps.add("clear:$x,$y,${width}x$height") }
  override fun draw(still: ByteArray, x: Int, y: Int, blends: Boolean) {
    steps.add("draw:$x,$y,blend=$blends,riff=${String(still, 0, 4, Charsets.ISO_8859_1)},webp=${String(still, 8, 4, Charsets.ISO_8859_1)}")
  }
}

@RunWith(MediaLayerTestRunner::class)
class RestFrameDecoderTest {
  @Test
  fun inv_image_53_theRiffOfAnAnimatedWebpGivesItsFramesAndDelays() {
    assertTrue(WebpAnimation.isAnimated(animatedWebp))
    val animation = WebpAnimation.parse(animatedWebp)

    assertEquals(3, animation.frames.size)
    assertEquals(300, animation.durationMs)
    assertEquals(16, animation.width)
    assertEquals(16, animation.height)
    assertFalse(WebpAnimation.isAnimated(animatedGif))
  }

  @Test
  fun inv_image_53_theFramesOfAnAnimatedWebpAreComposedInOrderFromTheirStillFiles() {
    val surface = RecordingSurface()

    WebpAnimation.parse(animatedWebp).compose(surface)

    val draws = surface.steps.filter { it.startsWith("draw:") }
    assertEquals(
      listOf("draw:0,0,blend=false,riff=RIFF,webp=WEBP", "draw:4,0,blend=true,riff=RIFF,webp=WEBP", "draw:8,0,blend=true,riff=RIFF,webp=WEBP"),
      draws
    )
  }

  @Test
  fun inv_image_53_theFormatOfASourceDecidesWhichDecoderReadsItsLastFrame() {
    assertEquals(AnimatedFormat.Webp, animatedFormat(animatedWebp))
    assertEquals(AnimatedFormat.Gif, animatedFormat(animatedGif))
    assertNull(animatedFormat(byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8)))
    assertNull(lastFrame(byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8)))
  }

  private fun riff(vararg chunks: ByteArray): ByteArray {
    val body = chunks.fold("WEBP".toByteArray()) { all, chunk -> all + chunk }
    return "RIFF".toByteArray() + java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.LITTLE_ENDIAN).putInt(body.size).array() + body
  }

  private fun chunk(type: String, size: Int, payload: Int = 0) =
    type.toByteArray() + java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.LITTLE_ENDIAN).putInt(size).array() + ByteArray(payload)

  @Test(timeout = 5_000)
  fun inv_image_84_a_chunk_size_of_minus_eight_ends_the_walk_instead_of_looping() {
    val corrupt = riff(chunk("VP8X", -8, 12))
    assertFalse(WebpAnimation.isAnimated(corrupt))
    assertEquals(0, WebpAnimation.parse(corrupt).frames.size)
  }

  @Test(timeout = 5_000)
  fun inv_image_85_sizes_are_unsigned_and_a_chunk_beyond_the_file_is_not_a_frame() {
    assertFalse(WebpAnimation.isAnimated(riff(chunk("ANMF", -1, 20))))
    assertFalse(WebpAnimation.isAnimated(riff(chunk("ANMF", 100, 20))))
    assertEquals(0, WebpAnimation.parse(riff(chunk("ANMF", 4, 4))).frames.size)
    assertTrue(WebpAnimation.isAnimated(riff(chunk("VP8X", 10, 10), chunk("ANMF", 16, 16))))
  }
}

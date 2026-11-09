package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.drawable.Drawable
import androidx.test.core.app.ApplicationProvider
import coil3.size.Scale
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.Base64
import java.util.concurrent.Executor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private val fixtureWebp = Base64.getDecoder().decode("UklGRrQAAABXRUJQVlA4WAoAAAACAAAADwAADwAAQU5JTQYAAAAAAAAAAABBTk1GKAAAAAAAAAAAAAcAAAcAAGQAAANWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAIAAAAAAAcAAAcAAGQAAAFWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAQAAAAAAAcAAAcAAGQAAABWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQA=")
private val fixtureGif = Base64.getDecoder().decode("R0lGODlhEAAQAIEAAAAAAP8AAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQJCgAAACwAAAAAEAAQAAAIMAADCBwoEIDBgwYJEkSIUOFAhgcdFoQIQGIAihUlYrS4USPGjyBDihxJsqTJkyADAgAh+QQJCgAAACwEAAAACAAIAIEAAAD/AAAAAAAAAAAIDwADCBxIsKDBgwgTKkwYEAAh+QQJCgAAACwIAAAACAAIAIEAAAD/AAAAAAAAAAAIDwADCBxIsKDBgwgTKkwYEAA7")

private val ratedPage = object : MediaSource {
  override val url = "https://o.test/rated.gif"
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
  override val size = 24 to 24
}

private class Stage {
  var now = 1000L
  val painted = mutableListOf<Int>()
  val timeline = AnimationTimeline(intArrayOf(100, 200, 300))
  private val scheduled = mutableListOf<Pair<Runnable, Long>>()
  private val callback = object : Drawable.Callback {
    override fun invalidateDrawable(who: Drawable) {}
    override fun scheduleDrawable(who: Drawable, what: Runnable, whenMs: Long) { scheduled.add(what to whenMs) }
    override fun unscheduleDrawable(who: Drawable, what: Runnable) { scheduled.removeAll { it.first === what } }
  }

  val wakeups: List<Long> get() = scheduled.map { it.second }

  fun animation(rate: Double, repeats: Repeats = Repeats.Forever): RatedAnimation =
    RatedAnimation(timeline, { index, _ -> painted.add(index) }, bitmap(), bitmap(), rate, repeats, { now }, Executor { it.run() }).also { it.callback = callback }

  fun at(elapsed: Long) {
    now = 1000 + elapsed
    fire()
    fire()
  }

  fun fire() {
    val due = scheduled.toList()
    scheduled.clear()
    due.forEach { it.first.run() }
  }

  private fun bitmap(): Bitmap = Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888)
}

@RunWith(MediaLayerTestRunner::class)
class RatedAnimationTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()

  @Test
  fun inv_image_76_theFrameShownAtTimeTWithRateRIsTheFrameOfTheUnscaledAnimationAtTTimesR() {
    val cases = listOf(
      Triple(1.0, 99L, 0), Triple(1.0, 100L, 1), Triple(1.0, 299L, 1), Triple(1.0, 300L, 2),
      Triple(0.5, 100L, 0), Triple(0.5, 200L, 1), Triple(0.5, 600L, 2), Triple(0.5, 1200L, 0),
      Triple(2.0, 50L, 1), Triple(2.0, 150L, 2), Triple(2.0, 300L, 0), Triple(2.0, 400L, 1),
      Triple(3.0, 100L, 2), Triple(3.0, 250L, 1)
    )
    cases.forEach { (rate, elapsed, expected) ->
      val scaledStage = Stage()
      val scaled = scaledStage.animation(rate)
      scaled.start()
      scaledStage.at(elapsed)
      val plainStage = Stage()
      val plain = plainStage.animation(1.0)
      plain.start()
      plainStage.at((elapsed * rate).toLong())

      assertEquals("rate $rate at $elapsed ms", expected, scaled.shownFrame)
      assertEquals("rate $rate at $elapsed ms against the unscaled animation at ${(elapsed * rate).toLong()} ms", plain.shownFrame, scaled.shownFrame)
    }
  }

  @Test
  fun inv_image_76_aRateNotAboveZeroPlaysAtOne() {
    listOf(0.0, -2.0, Double.NaN, Double.POSITIVE_INFINITY).forEach { assertEquals(1.0, playbackRateOf(it), 0.0) }
    assertEquals(0.5, playbackRateOf(0.5), 0.0)
    val stage = Stage()
    val animation = stage.animation(0.0)
    animation.start()
    stage.at(250)

    assertEquals(1.0, animation.rate, 0.0)
    assertEquals(1, animation.shownFrame)
  }

  @Test
  fun inv_image_77_aNewRateChangesTheSpeedOfTheRunningAnimationFromWhereItIs() {
    val stage = Stage()
    val animation = stage.animation(1.0)
    animation.start()
    stage.at(100)
    assertEquals(1, animation.shownFrame)

    animation.rateTo(2.0)
    stage.at(200)

    assertEquals("100 ms at rate 1 and 100 ms at rate 2 are 300 ms of the animation", 2, animation.shownFrame)
    assertEquals(listOf(1, 2), stage.painted)
    assertTrue(playsAtRate(0.5, null))
    assertTrue("a change between two rates keeps the request", playsAtRate(2.0, null) == playsAtRate(0.5, null))
    assertFalse(playsAtRate(1.0, null))
    assertFalse(playsAtRate(0.0, null))
    assertFalse("a resting image has no rate", playsAtRate(0.5, RestFrame.First))
  }

  @Test
  fun inv_image_78_aStoppedAnimationWakesNothingAndARunningOneWakesAtTheScaledEndOfItsFrame() {
    val stage = Stage()
    val animation = stage.animation(0.5)
    animation.start()
    assertEquals("the first frame ends after 100 ms of the animation, 200 ms of the clock", listOf(1200L), stage.wakeups)

    animation.stop()
    stage.now += 10_000
    stage.fire()

    assertTrue("a stopped animation scheduled work", stage.wakeups.isEmpty())
    assertTrue("a stopped animation painted a frame", stage.painted.isEmpty())
  }

  @Test
  fun inv_image_78_theRepeatsEndAtTheScaledEndOfTheLastLoopOnTheLastFrame() {
    val stage = Stage()
    val animation = stage.animation(2.0, Repeats.Times(1))
    var ended = 0
    animation.onFinished { ended += 1 }
    animation.start()

    stage.at(299)
    assertEquals(0, ended)
    assertTrue(animation.isRunning)

    stage.at(300)
    assertEquals(1, ended)
    assertFalse(animation.isRunning)
    assertEquals("the animation rests on its last frame", 2, animation.shownFrame)
  }

  @Test
  fun inv_image_79_aRateOfOneKeepsTheRequestPathAndAnotherRateDecodesWithTheScaledClock() {
    val plain = imageRequest(context, ratedPage, null, Scale.FIT, layer, admit = freeAdmit)
    val one = imageRequest(context, ratedPage, null, Scale.FIT, layer, admit = freeAdmit, rate = 1.0)
    val half = imageRequest(context, ratedPage, null, Scale.FIT, layer, admit = freeAdmit, rate = 0.5)
    val resting = imageRequest(context, ratedPage, null, Scale.FIT, layer, rest = RestFrame.Last, admit = freeAdmit, rate = 0.5)

    assertNull(one.decoderFactory)
    assertEquals(plain.data, one.data)
    assertTrue(half.decoderFactory is RatedAnimationDecoder.Factory)
    assertTrue("a resting image decodes its rest frame at any rate", resting.decoderFactory is RestFrameDecoder.Factory)
    val key = com.noma4i.medialayer.PixelSize(24u, 24u)
    assertNotEquals(imageMemoryKey("k", key, Scale.FIT), imageMemoryKey("k", key, Scale.FIT, rated = true))
    assertFalse(
      "the repeats of a rated image count in the drawable, not in Coil",
      imageRequest(context, ratedPage, null, Scale.FIT, layer, 0L, null, Repeats.Times(3), freeAdmit, null, 0.5).extras.asMap().values.contains(2)
    )
  }

  @Test
  fun inv_image_79_aGifGivesItsFrameDelaysAndTheMovieReadsTheSameTable() {
    val gif = gifFrames(fixtureGif)
    assertNotNull(gif)
    assertEquals(listOf(100, 100, 100), gif!!.delaysMs.toList())

    val fast = fixtureGif.copyOf()
    val control = (0 until fast.size - 2).first { fast[it] == 0x21.toByte() && fast[it + 1] == 0xF9.toByte() && fast[it + 2] == 4.toByte() }
    fast[control + 4] = 1
    fast[control + 5] = 0
    val rewritten = gifFrames(fast)!!

    assertEquals("a delay under 20 ms reads 100 ms", listOf(100, 100, 100), rewritten.delaysMs.toList())
    assertEquals("the movie gets the rewritten delay", 10, rewritten.bytes[control + 4].toInt())
    assertEquals("the source bytes stay as they came", 1, fast[control + 4].toInt())
    val controls = (0 until fixtureGif.size - 2).filter { fixtureGif[it] == 0x21.toByte() && fixtureGif[it + 1] == 0xF9.toByte() && fixtureGif[it + 2] == 4.toByte() }
    assertNull("one frame is not an animation", gifFrames(fixtureGif.copyOf(controls[1]) + 0x3B.toByte()))
  }

  @Test
  fun inv_image_79_anAnimatedWebpPaintsItsFirstFrameAtOnceAndAStillGoesToTheNextDecoder() {
    val animation = ratedAnimation(fixtureWebp, 1.0, 0.5, Repeats.Forever, Executor { it.run() })

    assertNotNull(animation)
    assertEquals(0.5, animation!!.rate, 0.0)
    assertEquals(16, animation.intrinsicWidth)
    assertEquals(16, animation.intrinsicHeight)
    assertNull(ratedAnimation(byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14), 1.0, 0.5, Repeats.Forever))
  }
}

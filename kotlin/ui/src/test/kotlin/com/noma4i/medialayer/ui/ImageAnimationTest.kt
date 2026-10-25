package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.ColorFilter
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.Drawable
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DataSource
import coil3.fetch.Fetcher
import coil3.fetch.ImageFetchResult
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private class ControlledAnimation : Drawable(), RepeatingAnimation {
  val log = CopyOnWriteArrayList<String>()
  private var running = false
  val starts: Int get() = log.count { it == "start" }
  private var finish: () -> Unit = {}
  override fun onFinished(callback: () -> Unit) { finish = callback }
  fun end() { running = false; finish() }
  override fun start() { running = true; log.add("start") }
  override fun stop() { running = false }
  override fun isRunning() = running
  override fun draw(canvas: Canvas) {}
  override fun setAlpha(alpha: Int) {}
  override fun setColorFilter(colorFilter: ColorFilter?) {}
  @Deprecated("Deprecated in Java")
  override fun getOpacity() = PixelFormat.TRANSLUCENT
  override fun getIntrinsicWidth() = 24
  override fun getIntrinsicHeight() = 24
}

private val page = object : MediaSource {
  override val url = "https://o.test/animated.webp"
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
  override val size = 24 to 24
}

@RunWith(MediaLayerTestRunner::class)
class ImageAnimationTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val layer = RecordingLayer()
  private val animation = ControlledAnimation()
  private val rests = CopyOnWriteArrayList<RestFrame?>()
  private val repeats = CopyOnWriteArrayList<Repeats>()
  private var gatedRest = CompletableDeferred<Unit>().also { it.complete(Unit) }
  private val loader = ImageLoader.Builder(context).components {
    add(Fetcher.Factory<MediaImageData> { data, _, _ ->
      Fetcher {
        rests.add(data.rest)
        repeats.add(data.repeats)
        if (data.rest != null) gatedRest.await()
        // The rest frame is a still: the real decoder returns a bitmap, never the running drawable.
        val image = if (data.rest == null) animation.asImage() else ColorDrawable(Color.RED).asImage()
        ImageFetchResult(image, false, DataSource.NETWORK)
      }
    })
  }.build()

  @After
  fun restoreAnimators() {
    Settings.Global.putFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
  }

  private fun setScale(scale: Float) {
    Settings.Global.putFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, scale)
    context.contentResolver.notifyChange(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), null)
  }

  private fun until(condition: () -> Boolean) {
    rule.waitForIdle()
    rule.waitUntil(5000) { looper.idle(); condition() }
  }

  @Test
  fun inv_image_51_aRestingAnimationIsNeverStartedAndPlaysWhenReleased() {
    var animationState by mutableStateOf(ImageAnimation(playing = false))
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = animationState) }
    }
    until { layer.calls.contains("visible:${page.url}:true") && rests.isNotEmpty() }
    rule.waitForIdle()
    assertEquals("a resting animation started", 0, animation.starts)

    animationState = ImageAnimation(playing = true)
    until { animation.isRunning }
    animationState = ImageAnimation(playing = false)
    until { !animation.isRunning }
  }

  @Test
  fun inv_image_51_aRunningAnimationStopsAtOnceWhenItRestsBeforeItsRestFrameArrives() {
    var animationState by mutableStateOf(ImageAnimation())
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = animationState) }
    }
    until { animation.isRunning }

    gatedRest = CompletableDeferred()
    animationState = ImageAnimation(playing = false)
    until { rests.size == 2 }
    until { !animation.isRunning }
    assertFalse("the animation ran on while its rest frame loaded", animation.isRunning)
    gatedRest.complete(Unit)
  }

  @Test
  fun inv_image_52_theRepeatsGoWithTheRequestThatDecodesTheAnimation() {
    var animationState by mutableStateOf(ImageAnimation(repeats = Repeats.Times(3)))
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = animationState) }
    }
    until { animation.isRunning }
    assertEquals(listOf<Repeats>(Repeats.Times(3)), repeats.toList())

    animationState = ImageAnimation(repeats = Repeats.Forever)
    until { repeats.size == 2 }
    assertEquals(listOf(Repeats.Times(3), Repeats.Forever), repeats.toList())
  }

  @Test
  fun inv_image_52_theRepeatsOfAnImageCountOneLessInTheDecoder() {
    val request = imageRequest(context, page, null, coil3.size.Scale.FIT, { "k" }, 0L, null, Repeats.Times(3))
    assertTrue(request.extras.asMap().values.contains(2))
    assertFalse(imageRequest(context, page, null, coil3.size.Scale.FIT, { "k" }).extras.asMap().values.contains(-1))
  }

  @Test
  fun inv_image_53_aRestingAnimationDecodesItsRestFrameAndAPlayingOneDoesNot() {
    var animationState by mutableStateOf(ImageAnimation(playing = false, rest = RestFrame.First))
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = animationState) }
    }
    until { rests.isNotEmpty() }
    assertEquals(listOf<RestFrame?>(RestFrame.First), rests.toList())

    animationState = ImageAnimation(playing = true)
    until { rests.size == 2 }
    assertEquals(listOf<RestFrame?>(RestFrame.First, null), rests.toList())
    until { animation.isRunning }
    val started = animation.starts

    animationState = ImageAnimation(playing = false, rest = RestFrame.Last)
    until { rests.size == 3 }
    assertEquals(listOf<RestFrame?>(RestFrame.First, null, RestFrame.Last), rests.toList())
    assertEquals("a resting animation started", started, animation.starts)
  }

  @Test
  fun inv_image_52_anAnimationThatEndedItsRepeatsRestsOnItsRestFrame() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = ImageAnimation(repeats = Repeats.Times(1), rest = RestFrame.Last)) }
    }
    until { animation.isRunning }
    assertEquals(listOf<RestFrame?>(null), rests.toList())
    val started = animation.starts

    animation.end()
    until { rests.size == 2 }
    assertEquals(listOf<RestFrame?>(null, RestFrame.Last), rests.toList())
    assertEquals("a finished animation started again", started, animation.starts)
  }

  @Test
  fun inv_image_54_theSystemSettingRestsTheAnimationUnlessTheAppAllowsIt() {
    assertFalse(ImageAnimation().rests(reduceMotion = false))
    assertTrue(ImageAnimation().rests(reduceMotion = true))
    assertFalse(ImageAnimation(ignoresReduceMotion = true).rests(reduceMotion = true))
    assertTrue(ImageAnimation(playing = false).rests(reduceMotion = false))
  }

  @Test
  fun inv_image_54_theRemoveAnimationsSettingRestsAShownAnimationAndLiftingItPlaysAgain() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp)) }
    }
    until { animation.isRunning }

    setScale(0f)
    until { !animation.isRunning }
    setScale(1f)
    until { animation.isRunning }
  }

  @Test
  fun inv_image_54_anAppThatAllowsAnimationKeepsPlayingWhileTheSettingRemovesIt() {
    setScale(0f)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(page, Modifier.size(50.dp), animation = ImageAnimation(ignoresReduceMotion = true)) }
    }
    until { animation.isRunning }
    assertEquals(listOf<RestFrame?>(null), rests.toList())
  }

  @Test
  fun inv_image_55_anAnimationOutsideTheViewportIsNeverStarted() {
    var x by mutableIntStateOf(5000)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        Box(Modifier.fillMaxSize()) { Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) } }
      }
    }
    rule.waitForIdle()
    looper.idleFor(java.time.Duration.ofMillis(1000))
    assertEquals(0, animation.starts)
    assertTrue(rests.isEmpty())

    x = 0
    until { animation.isRunning }
  }
}

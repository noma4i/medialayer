package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.ColorFilter
import android.graphics.PixelFormat
import android.graphics.drawable.Animatable
import android.graphics.drawable.Drawable
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
import androidx.lifecycle.Lifecycle
import coil.ImageLoader
import coil.decode.DataSource
import coil.fetch.DrawableResult
import coil.fetch.Fetcher
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private class CountingAnimation : Drawable(), Animatable {
  private var running = false
  override fun start() { running = true }
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

@RunWith(MediaLayerTestRunner::class)
class LoadedAnimationVisibilityTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  @Test
  fun inv_image_10_loadedAnimationStopsWhenItsImageLeavesTheViewport() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val layer = RecordingLayer()
    val animation = CountingAnimation()
    val loader = ImageLoader.Builder(context).components {
      add(Fetcher.Factory<MediaImageData> { _, _, _ ->
        Fetcher { DrawableResult(animation, false, DataSource.NETWORK) }
      })
    }.build()
    val page = object : MediaSource {
      override val url = "https://o.test/loaded.gif"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
      override val size = 24 to 24
    }
    val looper = shadowOf(android.os.Looper.getMainLooper())
    var x by mutableIntStateOf(0)
    rule.setContent {
      MediaLayerProvider(layer, loader) {
        Box(Modifier.fillMaxSize()) {
          Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) }
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); animation.isRunning && layer.calls.contains("visible:${page.url}:true") }
    x = 5000
    rule.waitForIdle()
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:${page.url}:false") }
    rule.waitForIdle()
    assertFalse("Animation still runs after confirmed visible=false", animation.isRunning)
    repeat(2) {
      x = 0
      rule.waitForIdle()
      rule.waitUntil(5000) { looper.idle(); animation.isRunning }
      rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
      rule.waitForIdle()
      assertFalse("Animation still runs below STARTED", animation.isRunning)
      rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
      rule.waitUntil(5000) { looper.idle(); animation.isRunning }
      x = 5000
      rule.waitForIdle()
      rule.waitUntil(5000) { looper.idle(); !animation.isRunning }
    }
  }

  @Test
  fun inv_image_10_ownersOfOneUrlPauseIndependentlyWithoutReloading() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val layer = RecordingLayer()
    val animations = CopyOnWriteArrayList<CountingAnimation>()
    val loader = ImageLoader.Builder(context).components {
      add(Fetcher.Factory<MediaImageData> { _, _, _ ->
        Fetcher {
          val animation = CountingAnimation()
          animations.add(animation)
          DrawableResult(animation, false, DataSource.NETWORK)
        }
      })
    }.build()
    val page = object : MediaSource {
      override val url = "https://o.test/shared.gif"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
      override val size = 24 to 24
    }
    val looper = shadowOf(android.os.Looper.getMainLooper())
    var x by mutableIntStateOf(0)
    var shown by mutableStateOf(true)
    rule.setContent {
      MediaLayerProvider(layer, loader) {
        Box(Modifier.fillMaxSize()) {
          if (shown) {
            Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) }
            MediaImage(page, Modifier.size(50.dp))
          }
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); animations.size == 2 && animations.all { it.isRunning } }
    x = 5000
    rule.waitForIdle()
    rule.waitUntil(5000) { looper.idle(); animations.count { it.isRunning } == 1 }
    assertFalse(layer.calls.contains("visible:${page.url}:false"))
    x = 0
    rule.waitForIdle()
    rule.waitUntil(5000) { looper.idle(); animations.all { it.isRunning } }
    assertEquals("Returning owners must reuse their loaded drawables", 2, animations.size)
    shown = false
    rule.waitForIdle()
    assertTrue("Disposal must stop both animations", animations.none { it.isRunning })
  }
}

package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PlaybackRoute
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class DeclaredSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class SourceKeyTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val factory = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper().also {
    it.mediaSourceFor = { FakeMediaSource() }
    it.bind(layer.playerEvents())
  }
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val looper = shadowOf(Looper.getMainLooper())

  @After
  fun release() {
    factory.release()
    loader.shutdown()
    looper.idle()
  }

  @Test
  fun inv_source_11_a_place_declared_in_any_accepted_form_follows_the_progress_of_the_core_key_and_reads_it_off_the_main_thread() {
    val forms = listOf(
      " https://o.test/a.mp4 " to "https://o.test/a.mp4",
      "/tmp/a b.mp4" to "file:///tmp/a%20b.mp4",
      "file:///tmp/c.mp4" to "file:///tmp/c.mp4"
    )
    val readOn = CopyOnWriteArrayList<Pair<String, Boolean>>()
    val reads: (MediaSource) -> String = { source ->
      readOn += source.url to (Looper.myLooper() == Looper.getMainLooper())
      forms.first { it.first == source.url }.second
    }
    forms.forEachIndexed { index, (_, canonical) ->
      val slot = factory.createSlot(index.toUInt())
      slot.load(canonical, PlaybackRoute(canonical, emptyMap(), null, false, false))
      slot.play()
    }
    var declared by mutableStateOf(forms.first().first)
    var reports = 0
    rule.setContent {
      CompositionLocalProvider(LocalSourceKey provides reads) {
        MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
          MediaVideo(DeclaredSource(declared, Kind.VIDEO), modifier = Modifier.size(80.dp), onProgress = { reports++ })
        }
      }
    }

    forms.forEachIndexed { index, (raw, _) ->
      if (index > 0) reports = 0
      rule.runOnIdle { declared = raw }
      rule.waitUntil(5000) { looper.idle(); reports >= 1 }
      assertTrue("$raw: no progress reached the place", reports >= 1)
    }
    assertEquals(forms.map { it.first }, readOn.map { it.first }.distinct())
    assertTrue("the key was read on the main thread: $readOn", readOn.none { it.second })
  }
}

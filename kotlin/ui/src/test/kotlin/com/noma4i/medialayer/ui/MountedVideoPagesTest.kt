package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private const val PAGES = 50

private data class PageSource(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.VIDEO
  override val headers: Map<String, String> get() = emptyMap()
}

private class CountingSlot : PlayerSlot, PlayerSlotFactory {
  val created = AtomicInteger()
  val calls = CopyOnWriteArrayList<String>()
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this).also { created.incrementAndGet() }
  override suspend fun apply(command: SlotCall) {
    if (command == SlotCall.Play) calls.add("play")
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

@RunWith(MediaLayerTestRunner::class)
class MountedVideoPagesTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = CountingSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> ImageFetchResult(pngBytes(), ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
    dir = Files.createTempDirectory("mounted-pages").toString(),
    fetcher = object : Fetcher {
      override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
    },
    settings = SharedPreferencesSettings(context, "mounted-pages"), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))
  private val looper = shadowOf(Looper.getMainLooper())

  @Test
  fun inv_mount_01_manyMountedVideoPagesCreateOnlyAsManyPlayersAsThereAreVisiblePages() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        Row(Modifier.horizontalScroll(rememberScrollState())) {
          repeat(PAGES) { MediaVideo(PageSource("file:///page-$it.mp4"), modifier = Modifier.width(320.dp).height(400.dp)) }
        }
      }
    }
    rule.waitUntil(10_000) { looper.idle(); slot.calls.isNotEmpty() }
    rule.waitForIdle()
    looper.idleFor(java.time.Duration.ofMillis(1000))

    assertTrue(slot.calls.isNotEmpty())
    assertEquals("created ${slot.created.get()} players for $PAGES mounted pages, 1 visible", 1, slot.created.get())
  }
}

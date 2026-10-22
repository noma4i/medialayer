package com.noma4i.medialayer.ui

import coil3.request.allowHardware
import kotlinx.coroutines.CompletableDeferred
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.layout.SubcomposeLayoutState
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.size
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class Page(override val url: String, override val kind: Kind, override val cover: String? = null, override val size: Pair<Int, Int>? = null) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaPrecomposeTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val fetched = CopyOnWriteArrayList<String>()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { url, _ -> fetched.add(url); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }).newBuilder().allowHardware(false).build() }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())

  private fun visibles(url: String) = layer.calls.filter { it.startsWith("visible:$url:") }

  private fun attaches(url: String) = (layer.playerEvents() as RecordingEvents).calls.filter { it.startsWith("attach:$url:") }

  private fun workFor(page: Page, measured: Boolean, content: @Composable () -> Unit) {
    val state = SubcomposeLayoutState()
    var measuring by mutableStateOf(false)
    var placed by mutableStateOf(false)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        SubcomposeLayout(state, Modifier.size(50.dp)) { constraints ->
          val children = if (placed || measuring) subcompose(page.url, content).map { it.measure(Constraints.fixed(constraints.maxWidth, constraints.maxHeight)) } else emptyList()
          layout(constraints.maxWidth, constraints.maxHeight) { if (placed) children.forEach { it.place(0, 0) } }
        }
      }
    }
    rule.runOnIdle { state.precompose(page.url, content) }
    if (measured) measuring = true
    rule.waitForIdle()
    repeat(20) { looper.idle(); Thread.sleep(25) }

    val work = "fetched=${fetched.size} visible=${visibles(page.url).size} attach=${attaches(page.url).size} placeholder=${layer.placeholders.count { it.first == page.url }}"
    assertEquals("fetched=0 visible=0 attach=0 placeholder=0", work)

    placed = true
    rule.waitUntil(5000) { looper.idle(); visibles(page.url) == listOf("visible:${page.url}:true") }
  }

  @Test
  fun aVideoDisposedBeforeItsFirstPlacementLeavesNoViewBehind() {
    val page = Page("https://o.test/never.mp4", Kind.VIDEO)
    val state = SubcomposeLayoutState()
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        SubcomposeLayout(state, Modifier.size(50.dp)) { constraints -> layout(constraints.maxWidth, constraints.maxHeight) {} }
      }
    }
    var handle: androidx.compose.ui.layout.SubcomposeLayoutState.PrecomposedSlotHandle? = null
    rule.runOnIdle { handle = state.precompose("ahead") { MediaVideo(page, modifier = Modifier.size(50.dp)) } }
    rule.waitForIdle()
    val whilePrecomposed = slots.players.liveViews
    rule.runOnIdle { handle?.dispose() }
    rule.waitForIdle()

    assertEquals("whilePrecomposed=0 afterDispose=0", "whilePrecomposed=$whilePrecomposed afterDispose=${slots.players.liveViews}")
  }

  @Test
  fun aPlacedVideoReleasesItsViewWhenItLeaves() {
    val page = Page("https://o.test/leaves.mp4", Kind.VIDEO)
    var shown by mutableStateOf(true)
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { if (shown) MediaVideo(page, modifier = Modifier.size(50.dp)) } }
    rule.waitUntil(5000) { looper.idle(); slots.players.liveViews == 1 }
    shown = false
    rule.waitUntil(5000) { looper.idle(); slots.players.liveViews == 0 }
  }

  @Test
  fun aComposedButNeverMeasuredImageStartsNoWork() {
    val page = Page("https://o.test/composed.png", Kind.IMAGE)
    workFor(page, measured = false) { MediaImage(page, Modifier.size(50.dp)) }
  }

  @Test
  fun aPrecomposedGifPageFetchesNothingSoNothingIsDecoded() {
    val page = Page("https://o.test/anim.gif", Kind.IMAGE)
    workFor(page, measured = true) { MediaImage(page, Modifier.size(50.dp)) }
  }

  @Test
  fun anUnplacedOwnerNeitherStartsNorReleasesTheWorkOfAPlacedOwnerOfTheSameKey() {
    val page = Page("https://o.test/shared.png", Kind.IMAGE)
    val state = SubcomposeLayoutState()
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        SubcomposeLayout(state, Modifier.size(50.dp)) { constraints ->
          val placed = subcompose("placed") { MediaImage(page, Modifier.size(50.dp)) }.map { it.measure(Constraints.fixed(constraints.maxWidth, constraints.maxHeight)) }
          layout(constraints.maxWidth, constraints.maxHeight) { placed.forEach { it.place(0, 0) } }
        }
      }
    }
    var handle: androidx.compose.ui.layout.SubcomposeLayoutState.PrecomposedSlotHandle? = null
    rule.runOnIdle { handle = state.precompose("ahead") { MediaImage(page, Modifier.size(50.dp)) } }
    rule.waitUntil(5000) { looper.idle(); fetched.toList() == listOf(page.url) }
    rule.runOnIdle { handle?.dispose() }
    rule.waitForIdle()

    assertEquals(listOf("visible:${page.url}:true"), visibles(page.url))
    assertEquals(listOf(page.url), fetched.toList())
  }

  @Test
  fun aPrecomposedUnsizedImageStartsNoWorkUntilItIsPlaced() {
    val page = Page("https://o.test/full.png", Kind.IMAGE)
    workFor(page, measured = true) { MediaImage(page, Modifier.size(50.dp)) }
    rule.waitUntil(5000) { looper.idle(); fetched.toList() == listOf(page.url) }
  }

  @Test
  fun aPrecomposedSizedImageStartsNoWorkUntilItIsPlaced() {
    val page = Page("https://o.test/tile.png", Kind.IMAGE, size = 24 to 24)
    workFor(page, measured = true) { MediaImage(page, Modifier.size(50.dp)) }
    rule.waitUntil(5000) { looper.idle(); fetched.toList() == listOf(page.url) }
  }

  @Test
  fun aPrecomposedVideoClaimsNoKeyAndAttachesNoViewUntilItIsPlaced() {
    val page = Page("https://o.test/clip.mp4", Kind.VIDEO, cover = "https://o.test/cover.jpg")
    workFor(page, measured = true) { MediaVideo(page, modifier = Modifier.size(50.dp)) }
    rule.waitUntil(5000) { looper.idle(); fetched.toList() == listOf("https://o.test/cover.jpg") }
    assertEquals(1, attaches(page.url).size)
  }
}

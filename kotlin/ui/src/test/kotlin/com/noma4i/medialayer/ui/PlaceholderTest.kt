package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Looper
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.neverEqualPolicy
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageOrigin
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.NoHandle
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.StateListener
import com.noma4i.medialayer.Subscription
import com.noma4i.medialayer.TonePreparation
import com.noma4i.medialayer.decodeBlurhash
import java.io.ByteArrayOutputStream
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private const val RED = "00TI:j"
private const val BLUE = "000036"
private const val GREEN = "0009dg"

private enum class Tint { RED, BLUE, GREEN, CLEAR, OTHER }

private sealed interface Event {
  data class Ask(val layer: String, val url: String, val input: String?, val ready: Boolean, val held: Boolean) : Event
  data class Returned(val layer: String, val url: String) : Event
  data class Decode(val hash: String, val onMain: Boolean) : Event
  data class Fetch(val url: String) : Event
  data class Composed(val url: String, val input: String?) : Event
  data class Published(val kind: String, val tint: Tint, val argb: Int) : Event
}

private class Events {
  private val items = CopyOnWriteArrayList<Event>()
  val all: List<Event> get() = items.toList()
  fun add(event: Event) { items.add(event) }
  fun has(match: (Event) -> Boolean) = items.any(match)
  fun count(match: (Event) -> Boolean) = items.count(match)
}

private class ScriptedLayer(val label: String, private val events: Events, private val answerFor: (MediaItem) -> String? = { it.blurHash }) : MediaLayer(NoHandle) {
  val mainCacheKeys = java.util.concurrent.atomic.AtomicInteger()
  val holds = ConcurrentLinkedQueue<CompletableDeferred<Unit>>()
  val listeners = CopyOnWriteArrayList<Pair<String, StateListener>>()
  private val recordingEvents = RecordingEvents()
  private val counters = RecordingCounters(CopyOnWriteArrayList(), CopyOnWriteArrayList())

  fun emit(key: String, state: MediaState) = listeners.filter { it.first == key }.forEach { it.second.changed(state) }

  override suspend fun placeholder(source: MediaItem, ready: Boolean): Placeholder {
    val answer = when {
      source.kind == Kind.IMAGE -> answerFor(source)?.let { Placeholder.BlurHash(it) } ?: Placeholder.None
      ready -> Placeholder.None
      else -> source.cover?.let { Placeholder.Cover(it) } ?: Placeholder.FirstFrame
    }
    val gate = holds.poll()
    events.add(Event.Ask(label, source.url, source.blurHash ?: source.cover, ready, gate != null))
    if (gate != null) withContext(NonCancellable) { gate.await() }
    events.add(Event.Returned(label, source.url))
    return answer
  }

  override fun cacheKey(url: String, headers: Map<String, String>): String {
    if (Looper.myLooper() == Looper.getMainLooper()) mainCacheKeys.incrementAndGet()
    return url
  }
  override fun imageStats(): com.noma4i.medialayer.ImageCounters = counters
  override fun playerEvents() = recordingEvents
  override fun openPlace(item: MediaItem): com.noma4i.medialayer.MediaPlace = NullPlace()
  override fun openVideoWindow(): com.noma4i.medialayer.MediaWindow = NullWindow()
  override fun openImageWindow(name: String, rowsPerPage: UInt, reversed: Boolean): com.noma4i.medialayer.MediaWindow = NullWindow()
  override fun subscribeState(key: String, listener: StateListener): Subscription { listeners.add(key to listener); return Subscription(NoHandle) }
  override fun setTonePreparation(preparation: TonePreparation) {}
  override suspend fun tonePixelsNeeded(source: MediaItem): Boolean = false
  override fun setMutedAll(muted: Boolean) {}
  override suspend fun fetchImage(url: String, headers: Map<String, String>): ImageFetchResult = throw kotlinx.coroutines.CancellationException("unused")
  override suspend fun playUrl(source: MediaItem): com.noma4i.medialayer.MediaPlayer = throw kotlinx.coroutines.CancellationException("unused")
  override fun close() {}
}

private class UrlKeyedImage(override val url: String, override var blurHash: String?) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
  override fun equals(other: Any?) = other is UrlKeyedImage && other.url == url
  override fun hashCode() = url.hashCode()
}

private class UrlKeyedClip(override val url: String, override var cover: String?) : MediaSource {
  override val kind: Kind get() = Kind.VIDEO
  override val headers: Map<String, String> get() = emptyMap()
  override fun equals(other: Any?) = other is UrlKeyedClip && other.url == url
  override fun hashCode() = url.hashCode()
}

private fun solidPng(color: Int): ByteArray {
  val out = ByteArrayOutputStream()
  Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888).apply { eraseColor(color) }.compress(Bitmap.CompressFormat.PNG, 100, out)
  return out.toByteArray()
}

@RunWith(MediaLayerTestRunner::class)
class PlaceholderTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val events = Events()
  private val never = CompletableDeferred<Unit>()
  private val looper = shadowOf(Looper.getMainLooper())
  private val mainPixelSizes = java.util.concurrent.atomic.AtomicInteger()
  private val loader by lazy {
    mediaImageLoader(context, ImageBytes { url, _ ->
      events.add(Event.Fetch(url))
      if (url.contains("cover")) ImageFetchResult(solidPng(if (url.contains("red")) Color.RED else if (url.contains("blue")) Color.BLUE else Color.GREEN), ImageOrigin.NETWORK)
      else { never.await(); ImageFetchResult(solidPng(Color.BLACK), ImageOrigin.NETWORK) }
    }, null, com.noma4i.medialayer.ImageCounters(), PixelSizeMemo { width, height, scale, declared, crop ->
      if (Looper.myLooper() == Looper.getMainLooper()) mainPixelSizes.incrementAndGet()
      com.noma4i.medialayer.imagePixelSize(width, height, scale, declared, crop)
    })
  }
  private val slots by lazy { ExoPlayerSlotFactory(context, startMuted = true).onMainLooper() }

  private fun until(what: String, condition: () -> Boolean) {
    try { rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); condition() } } catch (error: androidx.compose.ui.test.ComposeTimeoutException) {
      reportComposeTimeout(AssertionError("never became true: $what; events=${events.all}; nodes blurhash=${nodes("blurhash")} cover=${nodes("cover")}", error), "placeholder events=${events.all} mainPixelSizes=${mainPixelSizes.get()}")
    }
  }

  /** Robolectric legacy bitmaps keep the RGBA bytes of the decoder as one big-endian int, so the channels are read as RGBA here. */
  private fun tintOf(rgba: Int): Tint {
    val (r, g, b, a) = listOf(rgba ushr 24, (rgba ushr 16) and 0xFF, (rgba ushr 8) and 0xFF, rgba and 0xFF)
    return when {
      a < 30 -> Tint.CLEAR
      r > 180 && g < 90 && b < 90 -> Tint.RED
      b > 180 && r < 90 && g < 90 -> Tint.BLUE
      g > 180 && r < 90 && b < 90 -> Tint.GREEN
      else -> Tint.OTHER
    }
  }

  private val observer: (String, Any) -> Unit = { kind, shown ->
    when (shown) {
      is Bitmap -> shown.getPixel(shown.width / 2, shown.height / 2).let { events.add(Event.Published(kind, tintOf(it), it)) }
      is String -> events.add(Event.Published(kind, if (shown.contains("red")) Tint.RED else if (shown.contains("blue")) Tint.BLUE else if (shown.contains("green")) Tint.GREEN else Tint.OTHER, 0))
    }
  }

  private fun nodes(description: String) = rule.onAllNodesWithContentDescription(description).fetchSemanticsNodes().size
  private fun published(kind: String, tint: Tint) = events.has { it is Event.Published && it.kind == kind && it.tint == tint }

  private fun decoder(): (String) -> ByteArray = { hash ->
    events.add(Event.Decode(hash, runCatching { Looper.myLooper() == Looper.getMainLooper() }.getOrDefault(false)))
    decodeBlurhash(hash, 32u, 32u, 1f)
  }

  private fun held(layer: ScriptedLayer): CompletableDeferred<Unit> = CompletableDeferred<Unit>().also { layer.holds.add(it) }
  private fun askHeld(layer: ScriptedLayer) = events.has { it is Event.Ask && it.layer == layer.label && it.held }
  private fun returned(layer: ScriptedLayer, url: String) = events.has { it == Event.Returned(layer.label, url) }
  private fun oldWork(input: String) = events.count { (it as? Event.Decode)?.hash == input || (it as? Event.Fetch)?.url == input }

  private fun show(source: MediaSource, layer: ScriptedLayer) {
    rule.setContent {
      CompositionLocalProvider(LocalBlurDecoder provides decoder(), LocalPlaceholderObserver provides observer) {
        MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
          if (source.kind == Kind.VIDEO) MediaVideo(source, modifier = Modifier.size(50.dp)) else MediaImage(source, Modifier.size(50.dp))
        }
      }
    }
  }

  private fun showWithStill(source: MediaSource, layer: ScriptedLayer, asked: MutableList<String>) {
    val still = LocalStill { url, _, _ ->
      asked.add(url)
      Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888)
    }
    rule.setContent {
      CompositionLocalProvider(LocalLocalStill provides still) {
        MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaVideo(source, modifier = Modifier.size(50.dp)) }
      }
    }
  }

  @Test
  fun inv_placeholder_11_aLocalVideoWithoutACoverShowsItsFirstFrameUntilItIsRendered() {
    val layer = ScriptedLayer("A", events)
    val clip = UrlKeyedClip("file:///picked/clip.mp4", null)
    val asked = CopyOnWriteArrayList<String>()
    showWithStill(clip, layer, asked)
    until("first frame on the node") { nodes("firstframe") == 1 }

    assertEquals(listOf(clip.url), asked.toList())
    rule.runOnIdle { layer.emit(clip.url, MediaState.Rendered) }
    until("first frame gone with the rendered frame") { nodes("firstframe") == 0 }
  }

  @Test
  fun inv_placeholder_11_aRemoteVideoWithoutACoverDrawsNoStill() {
    val layer = ScriptedLayer("A", events)
    val asked = CopyOnWriteArrayList<String>()
    showWithStill(UrlKeyedClip("https://o.test/remote.mp4", null), layer, asked)
    until("the answer returned") { returned(layer, "https://o.test/remote.mp4") }
    rule.waitForIdle()

    assertEquals(0, nodes("firstframe"))
    assertTrue(asked.isEmpty())
  }

  @Test
  fun inv_placeholder_blurHashDecodesOffTheMainThreadAndIsShown() {
    val layer = ScriptedLayer("A", events)
    rule.setContent { CompositionLocalProvider(LocalBlurDecoder provides decoder(), LocalPlaceholderObserver provides observer) { MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(UrlKeyedImage("https://o.test/one.png", RED), Modifier.size(50.dp)) } } }

    until("red blur published to the node") { published("blurhash", Tint.RED) && nodes("blurhash") == 1 }

    assertTrue(events.has { it == Event.Decode(RED, false) })
    assertFalse(events.has { it == Event.Decode(RED, true) })
  }

  private fun replacing(first: MediaSource, second: MediaSource, control: MediaSource, video: Boolean, oldInput: String, secondLayer: Boolean = false) {
    val description = if (video) "cover" else "blurhash"
    val kind = description
    val a = ScriptedLayer("A", events)
    val b = ScriptedLayer("B", events) { if (it.url.contains("control")) it.blurHash else BLUE }
    val gate = held(a)
    val newLayer = if (secondLayer) b else a
    val newGate = held(newLayer)
    var source by mutableStateOf(first, neverEqualPolicy())
    var layer by mutableStateOf(a)
    var recompositionTrigger by mutableStateOf(50)
    rule.setContent {
      CompositionLocalProvider(LocalBlurDecoder provides decoder(), LocalPlaceholderObserver provides observer) {
        MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
          events.add(Event.Composed(source.url, source.blurHash ?: source.cover))
          if (video) MediaVideo(source, modifier = Modifier.size(recompositionTrigger.dp)) else MediaImage(source, Modifier.size(recompositionTrigger.dp))
        }
      }
    }
    until("old ask held") { askHeld(a) }
    if (video) until("old cover shown") { events.count { it is Event.Published && it.kind == kind && it.tint == Tint.RED } >= 1 }
    val oldShownBefore = events.count { it is Event.Published && it.kind == kind && it.tint == Tint.RED }
    val oldWorkBefore = oldWork(oldInput)

    source = second
    recompositionTrigger = 51
    if (secondLayer) layer = b
    until("new ask held") { events.count { it is Event.Ask && it.layer == newLayer.label && it.held } >= (if (secondLayer) 1 else 2) }
    if (video) {
      assertEquals("old cover shown after the replacement", oldShownBefore, events.count { it is Event.Published && it.kind == kind && it.tint == Tint.RED })
    } else {
      assertEquals("old picture shown after the replacement", 0, nodes(description))
    }

    newGate.complete(Unit)
    until("new picture on the node") { published(kind, Tint.BLUE) && nodes(description) == 1 }

    gate.complete(Unit)
    until("old answer returned") { returned(a, first.url) }
    source = control
    recompositionTrigger = 52
    until("control picture on the node") { published(kind, Tint.GREEN) && nodes(description) == 1 }

    assertEquals("old input worked after replacement", oldWorkBefore, oldWork(oldInput))
    assertEquals("old picture published after the replacement", oldShownBefore, events.count { it is Event.Published && it.kind == kind && it.tint == Tint.RED })
    assertEquals(0, a.mainCacheKeys.get() + b.mainCacheKeys.get())
    assertEquals(0, mainPixelSizes.get())
  }

  @Test
  fun inv_placeholder_anImageReplacedByUrlNeverShowsOrDecodesTheOldAnswer() =
    replacing(UrlKeyedImage("https://o.test/a.png", RED), UrlKeyedImage("https://o.test/b.png", BLUE), UrlKeyedImage("https://o.test/control.png", GREEN), false, RED)

  @Test
  fun inv_placeholder_anImageWhoseBlurHashChangesUnderTheSameEqualsNeverShowsTheOldOne() =
    replacing(UrlKeyedImage("https://o.test/same.png", RED), UrlKeyedImage("https://o.test/same.png", BLUE), UrlKeyedImage("https://o.test/control.png", GREEN), false, RED)

  @Test
  fun inv_placeholder_aVideoWhoseCoverChangesUnderTheSameEqualsNeverShowsTheOldOne() =
    replacing(UrlKeyedClip("https://o.test/v.mp4", "https://o.test/red-cover.png"), UrlKeyedClip("https://o.test/v.mp4", "https://o.test/blue-cover.png"), UrlKeyedClip("https://o.test/control.mp4", "https://o.test/green-cover.png"), true, "https://o.test/red-cover.png")

  @Test
  fun inv_placeholder_anImageMovedToAnotherMediaLayerNeverShowsTheOldAnswer() =
    replacing(UrlKeyedImage("https://o.test/layer.png", RED), UrlKeyedImage("https://o.test/layer.png", RED), UrlKeyedImage("https://o.test/control.png", GREEN), false, RED, secondLayer = true)

  @Test
  fun inv_placeholder_anUnmountedImageDoesNotDecodeALateAnswer() {
    val layer = ScriptedLayer("A", events)
    val gate = held(layer)
    var shown by mutableStateOf(true)
    var source by mutableStateOf<MediaSource>(UrlKeyedImage("https://o.test/gone.png", RED))
    rule.setContent {
      CompositionLocalProvider(LocalBlurDecoder provides decoder(), LocalPlaceholderObserver provides observer) {
        MediaLayerProvider(CompletableDeferred(layer), loader) { if (shown) MediaImage(source, Modifier.size(50.dp)) }
      }
    }
    until("ask held") { askHeld(layer) }
    shown = false
    rule.waitForIdle()
    gate.complete(Unit)
    until("answer returned") { returned(layer, "https://o.test/gone.png") }
    source = UrlKeyedImage("https://o.test/control.png", GREEN)
    shown = true
    until("control picture on the node") { published("blurhash", Tint.GREEN) && nodes("blurhash") == 1 }

    assertEquals(0, oldWork(RED))
    assertFalse(published("blurhash", Tint.RED))
  }

  @Test
  fun inv_placeholder_theSameVideoKeepsItsCoverUntilTheNextStepAnswers() {
    val layer = ScriptedLayer("A", events)
    val clip = UrlKeyedClip("https://o.test/steps.mp4", "https://o.test/red-cover.png")
    show(clip, layer)
    until("red cover on the node") { published("cover", Tint.RED) && nodes("cover") == 1 }
    val gate = held(layer)

    rule.runOnIdle { layer.emit(clip.url, MediaState.Rendered) }
    until("rendered ask held") { events.has { it is Event.Ask && it.ready && it.held } }

    assertEquals(1, nodes("cover"))
    gate.complete(Unit)
    until("cover gone after the rendered answer") { nodes("cover") == 0 }
  }

  @Test
  fun inv_placeholder_10_aReadyVideoKeepsItsCoverUntilTheFirstFrameIsRendered() {
    val layer = ScriptedLayer("A", events)
    val clip = UrlKeyedClip("https://o.test/frame.mp4", "https://o.test/red-cover.png")
    show(clip, layer)
    until("red cover on the node") { published("cover", Tint.RED) && nodes("cover") == 1 }

    rule.runOnIdle { layer.emit(clip.url, MediaState.Ready) }
    rule.waitForIdle()
    looper.idleFor(java.time.Duration.ofMillis(300))

    assertEquals(1, nodes("cover"))
    assertFalse(events.has { it is Event.Ask && it.ready })
    rule.runOnIdle { layer.emit(clip.url, MediaState.Rendered) }
    until("cover gone with the rendered frame") { nodes("cover") == 0 }
  }

  @Test
  fun inv_placeholder_10_aPageThatLeavesAndReturnsAfterItsFrameWasShownChangesNoCover() {
    org.robolectric.shadows.ShadowLog.setLoggable(MediaTrace.TAG, android.util.Log.VERBOSE)
    MediaTrace.install()
    try {
      val layer = ScriptedLayer("A", events)
      val clip = UrlKeyedClip("https://o.test/return.mp4", "https://o.test/red-cover.png")
      var x by mutableStateOf(0)
      rule.setContent {
        CompositionLocalProvider(LocalPlaceholderObserver provides observer) {
          MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
            androidx.compose.foundation.layout.Box(Modifier.offset { IntOffset(x, 0) }) { MediaVideo(clip, modifier = Modifier.size(50.dp)) }
          }
        }
      }
      until("red cover on the node") { published("cover", Tint.RED) && nodes("cover") == 1 }
      rule.runOnIdle { layer.emit(clip.url, MediaState.Rendered) }
      until("cover gone with the rendered frame") { nodes("cover") == 0 }
      val tail = clip.url.takeLast(24)
      fun changes() = org.robolectric.shadows.ShadowLog.getLogsForTag(MediaTrace.TAG).map { it.msg.substringAfter(' ').substringAfter(' ') }.filter { it.startsWith("view placeholder $tail") }
      val shown = changes()

      x = 5_000
      rule.waitForIdle()
      looper.idleFor(java.time.Duration.ofMillis(300))
      x = 0
      rule.waitForIdle()
      looper.idleFor(java.time.Duration.ofMillis(300))
      rule.runOnIdle { layer.emit(clip.url, MediaState.Loading) }
      looper.idleFor(java.time.Duration.ofMillis(300))

      assertEquals(listOf("view placeholder $tail cover", "view placeholder $tail none"), shown)
      assertEquals(shown, changes())
      assertEquals(0, nodes("cover"))
    } finally {
      MediaTrace.reset()
      org.robolectric.shadows.ShadowLog.reset()
    }
  }
}

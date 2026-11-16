@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import kotlinx.coroutines.CompletableDeferred
import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.ui.layout.ContentScale
import coil3.memory.MemoryCache
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.toItem
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private data class Clip(override val url: String, override val cover: String? = null) : MediaSource {
  override val kind: Kind get() = Kind.VIDEO
  override val headers: Map<String, String> get() = emptyMap()
}

private data class Photo(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = mapOf("Authorization" to "token")
  override val size: Pair<Int, Int> get() = 24 to 24
}

private data class Plain(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private data class BlurredPhoto(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
  override val blurHash: String get() = "LEHV6nWB2yk8pyo0adR*.7kCMdnj"
}

private data class Note(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.AUDIO
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaComposeTest {
  @get:Rule val rule = createComposeRule()

  private fun settled() { rule.waitForIdle(); drainFfi() }

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val events = RecordingEvents()
  private val layer = RecordingLayer(events)
  private val requested = CopyOnWriteArrayList<Pair<String, Map<String, String>>>()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { url, headers -> requested.add(url to headers); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }) }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()

  @Test
  fun aDeclaredPlaceTellsTheCoreOnlyWhenSomethingChangesAndWithdrawsOnLeave() {
    var playing by mutableStateOf(false)
    var unrelated by mutableIntStateOf(0)
    var shown by mutableStateOf(true)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (shown) MediaVideo(Clip("https://o.test/a.mp4"), playing = playing, muted = true, modifier = Modifier.size((10 + unrelated).dp)) { Text("tick") }
      }
    }
    val looper = org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper())
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("muted:https://o.test/a.mp4:true") }
    val declared = layer.calls.toList()

    unrelated++
    rule.waitForIdle()
    assertEquals(declared, layer.calls.toList())

    playing = true
    rule.waitUntil(5000) { looper.idle(); layer.calls.size == declared.size + 1 }
    assertEquals(declared + "playing:https://o.test/a.mp4:true", layer.calls.toList())

    shown = false
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("release:https://o.test/a.mp4") }
    assertEquals(listOf("release:https://o.test/a.mp4", "visible:https://o.test/a.mp4:false"), layer.calls.toList().takeLast(2))
    assertEquals(listOf("mutedAll:false", "visible:https://o.test/a.mp4:true", "muted:https://o.test/a.mp4:true"), declared)
    assertEquals(listOf("attach:https://o.test/a.mp4:1"), events.calls.toList())
  }

  @Test
  fun theAppWideMuteReachesTheCore() {
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { } }
    settled()

    Media.muted = true
    TestSignal.await("the core took the mute", 5_000) { layer.calls.contains("mutedAll:true") }
    Media.muted = false

    TestSignal.await("the core took the unmute", 5_000) { layer.calls.count { it == "mutedAll:false" } == 2 }
    settled()
    assertEquals(listOf("mutedAll:false", "mutedAll:true", "mutedAll:false"), layer.calls.toList())
  }

  @Test
  fun theCoreStateReachesTheContentAndTheCoverShowsUntilTheFirstFrame() {
    val clip = Clip("https://o.test/a.mp4", cover = "https://o.test/cover.jpg")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaVideo(clip, modifier = Modifier.size(100.dp)) { state -> Text(if (state is MediaState.Failed) "failed ${state.detail}" else "state") }
      }
    }
    rule.waitUntil(5000) { layer.listeners.isNotEmpty() && rule.onAllNodesWithContentDescription("cover").fetchSemanticsNodes().isNotEmpty() }
    rule.onNodeWithContentDescription("cover").assertIsDisplayed()

    layer.emit(clip.url, MediaState.Ready)
    rule.waitForIdle()
    rule.onNodeWithContentDescription("cover").assertIsDisplayed()
    layer.emit(clip.url, MediaState.Rendered)
    rule.waitUntil(5000) { rule.onAllNodesWithContentDescription("cover").fetchSemanticsNodes().isEmpty() }

    layer.emit(clip.url, MediaState.Failed("boom"))
    rule.waitUntil(5000) { rule.onAllNodesWithTextCount("failed boom") == 1 }
  }

  @Test
  fun inv_placeholder_05_a_video_cover_decodes_no_more_than_its_placement() {
    val stats = com.noma4i.medialayer.ImageCounters()
    val cover = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(2000, 2000), com.noma4i.medialayer.ImageOrigin.NETWORK) }, stats = stats)
    val clip = Clip("https://o.test/a.mp4", cover = "https://o.test/cover.jpg")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), cover, slots.players) { MediaVideo(clip, modifier = Modifier.size(50.dp)) }
    }
    rule.waitUntil(10000) { stats.snapshot().decodes == 1uL }

    val side = Math.round(50 * context.resources.displayMetrics.density).toULong()
    val stepped = (side + 7uL) / 8uL * 8uL
    assertEquals(side * side, stats.snapshot().placementPixels)
    org.junit.Assert.assertTrue("decoded=${stats.snapshot().decodedPixels} placement=${stats.snapshot().placementPixels}", stats.snapshot().decodedPixels <= stepped * stepped)
  }

  @Test
  fun aOneShotVideoDeclaresItselfWithoutLoopAndReportsTheFirstFrameAndEveryEnd() {
    var firstFrames = 0
    var ends = 0
    val clip = Clip("https://o.test/intro.mp4")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaVideo(clip, loop = false, onFirstFrame = { firstFrames++ }, onEnd = { ends++ }, modifier = Modifier.size(100.dp))
      }
    }
    rule.waitUntil(5000) { layer.listeners.isNotEmpty() }
    val states = listOf(MediaState.Loading, MediaState.Ready, MediaState.Rendered, MediaState.Ended, MediaState.Rendered, MediaState.Ended)

    states.forEach { state ->
      layer.emit(clip.url, state)
      rule.waitForIdle()
    }

    assertEquals(listOf(false), layer.items.filter { it.url == clip.url }.map { it.looping }.distinct())
    assertEquals(1 to 2, firstFrames to ends)
  }

  @Test
  fun inv_api_48_a_one_shot_video_reports_full_progress_before_its_end_even_when_the_end_outruns_every_sample() {
    val events = CopyOnWriteArrayList<String>()
    var mounted by mutableStateOf(true)
    val clip = Clip("https://o.test/note.mp4")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (mounted) MediaVideo(clip, loop = false, onEnd = { events += "end"; mounted = false }, onProgress = { events += "progress $it" }, modifier = Modifier.size(100.dp))
      }
    }
    rule.waitUntil(5000) { layer.listeners.isNotEmpty() }

    listOf(MediaState.Rendered, MediaState.Ended).forEach { state ->
      layer.emit(clip.url, state)
      rule.waitForIdle()
    }

    val end = events.indexOf("end")
    assertEquals("the end arrived without a full progress before it: $events", true, end > 0 && events[end - 1] == "progress 1.0")
  }

  @Test
  fun inv_android_21_a_one_shot_video_next_to_a_window_of_the_same_url_reports_the_end() {
    var ends = 0
    val clip = Clip("https://o.test/replay.mp4")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaWindow(data = { listOf(clip) }, source = { it.toItem().copy(looping = false) }, active = 0)
        MediaVideo(clip, playing = true, loop = false, onEnd = { ends++ }, modifier = Modifier.size(100.dp))
      }
    }
    rule.waitUntil(5000) { layer.listeners.isNotEmpty() }

    listOf(MediaState.Ready, MediaState.Rendered, MediaState.Ended).forEach { state ->
      layer.emit(clip.url, state)
      rule.waitForIdle()
    }

    assertEquals(listOf(false), layer.items.filter { it.url == clip.url }.map { it.looping }.distinct())
    assertEquals(1, ends)
  }

  @Test
  fun anAudioRateGoesToTheCoreAndAChangeOfTheParameterGoesAgain() {
    var rate by androidx.compose.runtime.mutableFloatStateOf(1.5f)
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaAudio(Note("https://o.test/n.m4a"), rate = rate) { } } }
    settled()

    rate = 2f
    settled()

    assertEquals(listOf("rate:https://o.test/n.m4a:1.5", "rate:https://o.test/n.m4a:2.0"), layer.calls.filter { it.startsWith("rate") })
  }

  @Test
  fun anImageLoadsThroughTheCoreAtTheDeclaredSizeAndReportsItsStateToTheCore() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaImage(Photo("https://o.test/p.png"), Modifier.size(50.dp))
      }
    }

    rule.waitUntil(5000) {
      org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
      layer.calls.any { it == "itemState:https://o.test/p.png:Ready" }
    }

    assertEquals(listOf("https://o.test/p.png" to mapOf("Authorization" to "token")), requested.toList())
    assertEquals(
      listOf("itemState:https://o.test/p.png:Loading", "itemState:https://o.test/p.png:Ready"),
      layer.calls.filter { it.startsWith("itemState") }
    )
  }

  private fun decodedWidthAndHeight(imageLoader: coil3.ImageLoader, key: String): Pair<Int, Int> =
    imageLoader.memoryCache!!.get(MemoryCache.Key(key))!!.image.let { (it as coil3.BitmapImage).bitmap }.let { it.width to it.height }

  private fun shown(imageLoader: coil3.ImageLoader, count: Int) {
    val looper = org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper())
    rule.waitUntil(5000) { looper.idle(); imageLoader.memoryCache!!.keys.size == count }
  }

  @Test
  @Config(qualifiers = "xxhdpi")
  fun inv_image_01_a_view_decodes_at_its_size_in_pixels_rounded_up_to_8_without_upscaling_the_source() {
    val sized = mediaImageLoader(context, ImageBytes { url, headers -> requested.add(url to headers); com.noma4i.medialayer.ImageFetchResult(pngBytes(400, 400), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), sized, slots.players) {
        Column {
          MediaImage(Plain("https://o.test/view.png"), Modifier.size(50.dp))
          MediaImage(Photo("https://o.test/declared.png"), Modifier.size(50.dp))
        }
      }
    }

    shown(sized, 2)

    assertEquals(152 to 152, decodedWidthAndHeight(sized, "https://o.test/view.png|152x152|FILL"))
    assertEquals(24 to 24, decodedWidthAndHeight(sized, "https://o.test/declared.png|24x24|FILL"))
    assertEquals(setOf("https://o.test/view.png", "https://o.test/declared.png"), requested.map { it.first }.toSet())
  }

  @Test
  @Config(qualifiers = "xxhdpi")
  fun inv_image_04_the_content_scale_picks_the_decoder_scale_and_keeps_the_decode_size() {
    val wide = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(400, 200), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), wide, slots.players) {
        Column {
          MediaImage(Plain("https://o.test/crop.png"), Modifier.size(50.dp), contentScale = ContentScale.Crop)
          MediaImage(Plain("https://o.test/fit.png"), Modifier.size(50.dp), contentScale = ContentScale.Fit)
        }
      }
    }

    shown(wide, 2)

    assertEquals(304 to 152, decodedWidthAndHeight(wide, "https://o.test/crop.png|152x152|FILL"))
    assertEquals(152 to 76, decodedWidthAndHeight(wide, "https://o.test/fit.png|152x152|FIT"))
  }

  @Test
  fun inv_image_05_an_image_in_the_decoder_memory_shows_in_the_first_composition_without_the_core() {
    val seen = CopyOnWriteArrayList<MediaState>()
    var visible by mutableStateOf(true)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (visible) MediaImage(Plain("https://o.test/again.png"), Modifier.size(50.dp)) { seen.add(it) }
      }
    }
    rule.waitUntil(5000) { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); seen.lastOrNull() == MediaState.Ready }
    val before = seen.size

    visible = false
    rule.waitForIdle()
    visible = true
    rule.waitForIdle()

    assertEquals(MediaState.Ready, seen[before])
    assertEquals(1, requested.size)
  }

  @Test
  fun theBlurHashShowsWhileTheImageLoadsAndGoesAwayWhenItIsReady() {
    val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
    val gated = mediaImageLoader(context, ImageBytes { _, _ -> gate.await(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    val looper = org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper())
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), gated, slots.players) { MediaImage(BlurredPhoto("https://o.test/b.png"), Modifier.size(50.dp)) } }

    rule.waitUntil(5000) { looper.idle(); rule.onAllNodesWithContentDescription("blurhash").fetchSemanticsNodes().isNotEmpty() }
    gate.complete(Unit)

    rule.waitUntil(5000) { looper.idle(); rule.onAllNodesWithContentDescription("blurhash").fetchSemanticsNodes().isEmpty() }
    assertEquals("itemState:https://o.test/b.png:Ready", layer.calls.last { it.startsWith("itemState") })
  }

  @Test
  fun anAudioPlaceTogglesPlayingThroughTheCore() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaAudio(Note("https://o.test/n.m4a")) { audio -> androidx.compose.foundation.layout.Column {
          Text(if (audio.isPlaying) "pause" else "play", Modifier.size(40.dp).clickableTap(audio::toggle))
          Text("seek", Modifier.size(40.dp).clickableTap { audio.seek(0.5f) })
        } }
      }
    }
    settled()

    rule.onNodeWithText("play").performClick()
    settled()
    rule.onNodeWithText("pause").performClick()
    settled()

    rule.onNodeWithText("seek").performClick()
    settled()

    assertEquals(
      listOf("playing:https://o.test/n.m4a:true", "playing:https://o.test/n.m4a:false"),
      layer.calls.filter { it.startsWith("playing") }.takeLast(2)
    )
    assertEquals("seek:https://o.test/n.m4a:0.5", layer.calls.last { it.startsWith("seek") })
  }

  @Test
  fun inv_audio_03_seek_on_an_unloaded_audio_shows_the_fraction_at_once_and_leaves_the_duration_to_the_core() {
    var control: AudioControl? = null
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaAudio(Note("https://o.test/n.m4a")) { control = it } } }
    rule.waitForIdle()

    rule.runOnIdle { control!!.seek(0.5f) }

    assertEquals(0.5f, control!!.progress)
    rule.waitUntil { layer.calls.any { it.startsWith("seek") } }
    assertEquals(listOf("seek:https://o.test/n.m4a:0.5"), layer.calls.filter { it.startsWith("seek") })
  }

  // mutation: start the video cover at None until the core answers the placeholder -> the TextureView shows black
  // between the output bind and the first frame (Android feed, owner complaint 2026-11-12, feed-cover rec-1)
  @Test
  fun aVideoShowsItsCoverFromTheFirstCompositionWhileTheCoreHasNotAnsweredYet() {
    layer.placeholderGate = CompletableDeferred()
    val clip = Clip("https://o.test/a.mp4", cover = "https://o.test/cover.jpg")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaVideo(clip, modifier = Modifier.size(100.dp)) }
    }
    rule.waitForIdle()
    assertEquals("the cover waited for the core", 1, rule.onAllNodesWithContentDescription("cover").fetchSemanticsNodes().size)

    layer.placeholderGate!!.complete(Unit)
    layer.emit(clip.url, MediaState.Rendered)
    rule.waitUntil(5000) { rule.onAllNodesWithContentDescription("cover").fetchSemanticsNodes().isEmpty() }
  }
}

private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onAllNodesWithTextCount(text: String): Int =
  onAllNodes(androidx.compose.ui.test.hasText(text)).fetchSemanticsNodes().size

private fun Modifier.clickableTap(action: () -> Unit): Modifier = this.clickable(onClick = action)

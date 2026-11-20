package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.request.allowHardware
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.OutgoingPhase
import com.noma4i.medialayer.OutgoingUpload
import com.noma4i.medialayer.RemoteResult
import com.noma4i.medialayer.testing.FakeOutgoingRemote
import com.noma4i.medialayer.testing.ManualOutgoingClock
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class OutgoingMediaTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val remote = FakeOutgoingRemote()
  private val clock = ManualOutgoingClock()
  private val store = File(context.cacheDir, "outgoing-test").apply { deleteRecursively(); mkdirs() }
  private val original = File(context.cacheDir, "original.png").apply { writeBytes(pngBytes()) }
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.CACHE) }, stats = layer.imageStats()).newBuilder().allowHardware(false).build() }

  @Before fun install() = Outgoing.install(remote, clock, store)
  @After fun uninstall() = Outgoing.reset()

  private fun declare(key: String, local: File? = original, blurHash: String? = null) = Outgoing.runtime.item(key) { OutgoingMedia(key, Kind.IMAGE, local, blurHash, Outgoing.runtime) }

  private fun waitFor(condition: () -> Boolean) = rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); condition() }

  private fun described(key: String): String? = rule.onNodeWithTag("outgoing-$key").fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription)

  @Test
  fun inv_outgoing_12_there_is_a_picture_in_every_frame_from_sending_to_ready_with_a_late_remote() {
    val item = declare("m12")
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { OutgoingMediaImage(item, Modifier.size(50.dp)) } }
    waitFor { rule.onAllNodes(hasContentDescription("cover")).fetchSemanticsNodes().isNotEmpty() }
    assertEquals("sending", described("m12"))

    item.forward(OutgoingUpload.Sent)
    item.remote = "https://cdn/m12.png"
    waitFor { described("m12") == "fetchingRemote" }
    rule.onNodeWithContentDescription("cover").assertExists()

    remote.answer("https://cdn/m12.png", RemoteResult.Complete)
    waitFor { described("m12") == "ready" && rule.onAllNodes(hasContentDescription("cover")).fetchSemanticsNodes().isEmpty() }
    rule.onNodeWithContentDescription("cover").assertDoesNotExist()
  }

  @Test
  fun inv_outgoing_13_the_blurred_cover_is_only_before_ready() {
    val item = declare("m13")
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { OutgoingMediaImage(item, Modifier.size(50.dp), cover = OutgoingCover.BlurredLocal(12.dp)) } }
    waitFor { rule.onAllNodes(hasContentDescription("cover-blurred")).fetchSemanticsNodes().isNotEmpty() }
    item.remote = "https://cdn/m13.png"
    remote.answer("https://cdn/m13.png", RemoteResult.Complete)
    waitFor { described("m13") == "ready" && rule.onAllNodes(hasContentDescription("cover-blurred")).fetchSemanticsNodes().isEmpty() }
    rule.onNodeWithContentDescription("cover-blurred").assertDoesNotExist()
  }

  @Test
  fun inv_outgoing_14_a_remote_failure_keeps_the_local_picture_and_retry_reaches_ready() {
    val item = declare("m14")
    val url = "https://cdn/m14.png"
    remote.answer(url, RemoteResult.Failed(500u, "boom"))
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { OutgoingMediaImage(item, Modifier.size(50.dp), failure = { Text("failed-overlay") }) } }
    item.remote = url
    waitFor { described("m14") == "failed.remote" }
    rule.onNodeWithContentDescription("cover").assertExists()
    rule.onNodeWithText("failed-overlay").assertExists()

    remote.clear(url)
    item.retry()
    waitFor { described("m14") == "fetchingRemote" }
    remote.answer(url, RemoteResult.Complete)
    waitFor { described("m14") == "ready" }
    assertEquals(2, remote.fetchCount(url))
  }

  @Test
  fun inv_outgoing_15_can_open_only_at_ready_and_the_app_rule_overrides() {
    val item = declare("m15")
    assertFalse(item.canOpen)
    item.remote = "https://cdn/m15.png"
    assertFalse(item.canOpen)
    item.openable = { it is OutgoingPhase.FetchingRemote }
    assertTrue(item.canOpen)
    item.openable = null
    remote.answer("https://cdn/m15.png", RemoteResult.Complete)
    waitFor { item.phase == OutgoingPhase.Ready }
    assertTrue(item.canOpen)
    item.openable = { false }
    assertFalse(item.canOpen)
  }

  @Test
  fun inv_outgoing_16_at_most_two_remote_fetches_run_at_once_with_ten_items() {
    val items = (0 until 10).map { declare("album$it", blurHash = null) }
    items.forEachIndexed { index, item -> item.remote = "https://cdn/album$index.png" }
    waitFor { remote.concurrent == 2 }
    assertEquals(2, remote.maxConcurrent)
    (0 until 10).forEach { remote.answer("https://cdn/album$it.png", RemoteResult.Complete) }
    waitFor { items.all { it.phase == OutgoingPhase.Ready } }
    assertEquals(2, remote.maxConcurrent)
  }

  @Test
  fun inv_outgoing_17_the_original_is_kept_for_the_same_key_and_released_at_ready() {
    val item = declare("m17")
    val kept = item.local!!
    assertTrue(kept.exists())
    assertTrue(kept.path.startsWith(store.path))
    assertEquals(kept, declare("m17", File(context.cacheDir, "other.png")).local)
    assertTrue(OutgoingStore(store).held("m17"))
    item.remote = "https://cdn/m17.png"
    remote.answer("https://cdn/m17.png", RemoteResult.Complete)
    waitFor { item.phase == OutgoingPhase.Ready }
    waitFor { !OutgoingStore(store).held("m17") }
  }

  @Test
  fun inv_outgoing_18_not_ready_is_asked_again_every_five_seconds_only_while_on_screen() {
    val item = declare("m18")
    val url = "https://cdn/m18.mp4"
    remote.answer(url, RemoteResult.NotReady)
    item.remote = url
    waitFor { remote.fetchCount(url) == 1 && item.phase is OutgoingPhase.FetchingRemote }
    clock.advance(5_000)
    assertEquals(1, remote.fetchCount(url))
    item.onScreen(true)
    waitFor { clock.pending == 1 }
    clock.advance(5_000)
    waitFor { remote.fetchCount(url) == 2 }
    item.onScreen(false)
  }
}

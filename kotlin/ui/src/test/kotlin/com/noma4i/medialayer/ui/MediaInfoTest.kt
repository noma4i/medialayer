package com.noma4i.medialayer.ui

import android.app.Application
import android.media.MediaMetadataRetriever
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSourceValue
import android.app.ActivityManager
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.testing.TestFetcher
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import org.robolectric.Shadows.shadowOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.shadows.ShadowMediaMetadataRetriever

@RunWith(MediaLayerTestRunner::class)
class MediaInfoTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()

  private fun clip(vararg metadata: Pair<Int, String>): String {
    val file = File.createTempFile("clip", ".mp4").apply { writeBytes(byteArrayOf(0)); deleteOnExit() }
    metadata.forEach { (key, value) -> ShadowMediaMetadataRetriever.addMetadata(file.path, key, value) }
    return "file://${file.path}"
  }

  @Test
  fun inv_info_03_aClipTurnedBy90DegreesGivesTheFrameAsTheScreenShowsIt() = runBlocking {
    val url = clip(
      MediaMetadataRetriever.METADATA_KEY_DURATION to "1500",
      MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH to "64",
      MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT to "32",
      MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION to "90",
    )

    val info = readMediaInfo(context, url)!!

    assertEquals(ImageSize(32u, 64u), info.frame)
    assertEquals(1500uL, info.durationMs)
  }

  @Test
  fun inv_info_04_anAudioFileGivesItsLengthAndNoFrame() = runBlocking {
    val info = readMediaInfo(context, clip(MediaMetadataRetriever.METADATA_KEY_DURATION to "2000"))!!

    assertNull(info.frame)
    assertEquals(2000uL, info.durationMs)
  }

  @Test
  fun inv_info_05_aFileWithoutMediaGivesNothingAndAMissingFileFails() = runBlocking {
    assertNull(readMediaInfo(context, clip()))
    val missing = MediaSourceValue.file("/tmp/${java.util.UUID.randomUUID()}.mp4", Kind.VIDEO).url

    assertThrows(MediaException.Fetch::class.java) { runBlocking { readMediaInfo(context, missing) } }
    Unit
  }

  private val owner = object : LifecycleOwner {
    override val lifecycle = LifecycleRegistry.createUnsafe(this)
  }

  private fun open(fetcher: TestFetcher): MediaLayer {
    val manager = context.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
    return openMediaLayer(context, fetcher = fetcher, settings = SharedPreferencesSettings(context), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()
  }

  @Test
  fun inv_info_06_aRemoteSourceGivesNothingWithoutARequest() {
    val fetcher = TestFetcher()
    val media = open(fetcher)

    assertNull(runBlocking { media.mediaInfo(MediaSourceValue.remote("https://o.test/a.mp4", Kind.VIDEO)) })
    assertTrue(fetcher.requests.isEmpty())
    media.close()
  }

  @Test
  fun inv_info_07_readingRunsOnTheDispatcherOfTheReaderAndNotOnTheCaller() {
    val media = open(TestFetcher())
    val url = clip(MediaMetadataRetriever.METADATA_KEY_DURATION to "900")
    val used = AtomicInteger()
    val recording = object : CoroutineDispatcher() {
      override fun dispatch(context: CoroutineContext, block: Runnable) {
        used.incrementAndGet()
        Dispatchers.IO.dispatch(context, block)
      }
    }
    val before = mediaInfoDispatcher
    mediaInfoDispatcher = recording
    try {
      val info = runBlocking { media.mediaInfo(MediaSourceValue(url, Kind.VIDEO)) }

      assertEquals(900uL, info?.durationMs)
      assertEquals(1, used.get())
    } finally {
      mediaInfoDispatcher = before
      media.close()
    }
  }
}

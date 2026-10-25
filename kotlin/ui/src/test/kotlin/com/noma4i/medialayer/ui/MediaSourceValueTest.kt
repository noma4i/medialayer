package com.noma4i.medialayer.ui

import android.app.Application
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSourceValue
import com.noma4i.medialayer.bundledSourceError
import com.noma4i.medialayer.toItem
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaSourceValueTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()

  @Test
  fun inv_source_01_the_value_cleans_blank_text_through_the_core() {
    val value = MediaSourceValue("  https://o.test/a.jpg\n", Kind.IMAGE, cover = "  ", blurHash = " LEHV6nWB ")

    assertEquals("https://o.test/a.jpg", value.url)
    assertNull(value.cover)
    assertEquals("LEHV6nWB", value.blurHash)
    assertEquals("https://o.test/a.jpg", value.toItem().url)
    assertNull(value.toItem().cover)
    assertEquals("", MediaSourceValue(" \t ", Kind.VIDEO).url)
    assertEquals(CacheStrategy.WINDOW, value.strategy)
  }

  @Test
  fun inv_source_02_a_path_a_file_and_a_uri_build_one_file_source() {
    val fromPath = MediaSourceValue.file("/tmp/a b.mp4", Kind.VIDEO)

    assertEquals("file:///tmp/a%20b.mp4", fromPath.url)
    assertEquals(fromPath, MediaSourceValue.file(File("/tmp/a b.mp4"), Kind.VIDEO))
    assertEquals(fromPath, MediaSourceValue.file(Uri.parse("file:///tmp/a%20b.mp4"), Kind.VIDEO))
    assertEquals("", MediaSourceValue.file("clips/a.mp4", Kind.VIDEO).url)
    assertEquals("", MediaSourceValue.file("https://o.test/a.mp4", Kind.VIDEO).url)
  }

  @Test
  fun inv_source_03_only_a_valid_http_or_https_text_is_a_remote_source() {
    assertEquals("https://o.test/v.mp4", MediaSourceValue.remote(" https://o.test/v.mp4 ", Kind.VIDEO).url)
    assertEquals("http://o.test/v.mp4", MediaSourceValue.remote("http://o.test/v.mp4", Kind.VIDEO).url)
    assertEquals("", MediaSourceValue.remote("ftp://o.test/v.mp4", Kind.VIDEO).url)
    assertEquals("", MediaSourceValue.remote("not a url", Kind.VIDEO).url)
    assertEquals(mapOf("X-Token" to "a"), MediaSourceValue.remote("https://o.test/v.mp4", Kind.VIDEO, headers = mapOf("X-Token" to "a")).headers)
  }

  @Test
  fun inv_source_04_a_bundled_image_is_refused_with_the_text_of_the_core() {
    val text = "a bundled source is a video or an audio, not an image"

    assertEquals(text, bundledSourceError(Kind.IMAGE))
    assertNull(bundledSourceError(Kind.VIDEO))
    assertNull(bundledSourceError(Kind.AUDIO))
    assertEquals(text, assertThrows(IllegalArgumentException::class.java) { MediaSourceValue.raw(context, 1, Kind.IMAGE) }.message)
    assertEquals(text, assertThrows(IllegalArgumentException::class.java) { MediaSourceValue.asset("a.jpg", Kind.IMAGE) }.message)
  }

  @Test
  fun inv_source_07_a_raw_resource_and_an_asset_become_resource_urls_the_player_reads_itself() {
    val id = R.raw.medialayer_test_clip
    val raw = MediaSourceValue.raw(context, id, Kind.VIDEO)

    assertEquals("android.resource://${context.packageName}/$id", raw.url)
    assertEquals(Kind.VIDEO, raw.kind)
    context.resources.openRawResourceFd(id).use { assertTrue(it.length > 0) }
    assertEquals("asset:///clips/intro.mp3", MediaSourceValue.asset("/clips/intro.mp3", Kind.AUDIO).url)
  }
}

package com.noma4i.medialayer.ui

import android.media.MediaExtractor
import android.media.MediaFormat
import androidx.media3.common.MimeTypes
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.defaultVideoLimits
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PrepareVideoDeviceTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context = instrumentation.targetContext

  private class Result(val mime: String, val longSide: Int, val durationUs: Long)

  private fun prepared(asset: String): Result = runBlocking {
    val source = File(context.cacheDir, asset).also { file ->
      instrumentation.context.assets.open(asset).use { input -> file.outputStream().use { input.copyTo(it) } }
    }
    val media = prepareVideoAt(context, source, defaultVideoLimits(), null)
    assertTrue("the clip had to be encoded", media.isNew)
    try {
      val extractor = MediaExtractor()
      try {
        extractor.setDataSource(media.file.absolutePath)
        val video = (0 until extractor.trackCount).map(extractor::getTrackFormat).first { it.getString(MediaFormat.KEY_MIME).orEmpty().startsWith("video/") }
        Result(
          video.getString(MediaFormat.KEY_MIME).orEmpty(),
          maxOf(video.getInteger(MediaFormat.KEY_WIDTH), video.getInteger(MediaFormat.KEY_HEIGHT)),
          video.getLong(MediaFormat.KEY_DURATION)
        )
      } finally {
        extractor.release()
      }
    } finally {
      media.file.delete()
      source.delete()
    }
  }

  private fun assertFits(result: Result) {
    assertEquals(MimeTypes.VIDEO_H264, result.mime)
    assertTrue("long side ${result.longSide}", result.longSide <= defaultVideoLimits().maxLongSide.toInt())
    assertEquals(3_000_000.0, result.durationUs.toDouble(), 300_000.0)
  }

  @Test
  fun inv_send_47_aFullHdClipIsPrepared() = assertFits(prepared("video-1080-sdr.mp4"))
}

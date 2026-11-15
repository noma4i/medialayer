package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Matrix
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class UprightVideoDeviceTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context = instrumentation.targetContext

  private class Stored(val width: Int, val height: Int, val rotation: Int, val durationMs: Long, val frame: Bitmap)

  private fun copy(asset: String): File = File(context.cacheDir, "upright-${System.nanoTime()}-$asset").also { file ->
    instrumentation.context.assets.open(asset).use { input -> file.outputStream().use { input.copyTo(it) } }
  }

  private fun read(file: File): Stored {
    val retriever = MediaMetadataRetriever()
    try {
      retriever.setDataSource(file.absolutePath)
      fun number(key: Int) = retriever.extractMetadata(key)?.toLong() ?: 0L
      return Stored(
        number(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH).toInt(),
        number(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT).toInt(),
        number(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION).toInt(),
        number(MediaMetadataRetriever.METADATA_KEY_DURATION),
        retriever.getFrameAtTime(500_000, MediaMetadataRetriever.OPTION_CLOSEST)!!
      )
    } finally {
      retriever.release()
    }
  }

  private fun shown(stored: Stored): Bitmap =
    if (stored.rotation != 0 && stored.frame.width == stored.width) {
      Bitmap.createBitmap(stored.frame, 0, 0, stored.frame.width, stored.frame.height, Matrix().apply { postRotate(stored.rotation.toFloat()) }, false)
    } else {
      stored.frame
    }

  private fun quadrants(frame: Bitmap): List<List<Int>> = listOf(0.25 to 0.25, 0.75 to 0.25, 0.25 to 0.75, 0.75 to 0.75).map { (x, y) ->
    val pixel = frame.getPixel((frame.width * x).toInt(), (frame.height * y).toInt())
    listOf(Color.red(pixel), Color.green(pixel), Color.blue(pixel))
  }

  private fun audioTracks(file: File): Int {
    val extractor = MediaExtractor()
    try {
      extractor.setDataSource(file.absolutePath)
      return (0 until extractor.trackCount).count { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME).orEmpty().startsWith("audio/") }
    } finally {
      extractor.release()
    }
  }

  @Test
  fun inv_capture_29_aRecordingWithATurnedTrackComesOutUprightInItsPixels() = runBlocking {
    val file = copy("rotated-recording.mp4")
    try {
      val before = read(file)
      assertTrue("fixture rotation ${before.rotation}", before.rotation == 90 || before.rotation == 270)
      val expected = shown(before)
      assertEquals(180 to 320, expected.width to expected.height)

      uprightVideoAt(context, file)

      val after = read(file)
      assertEquals(0, after.rotation)
      assertEquals(180 to 320, after.width to after.height)
      assertEquals(1, audioTracks(file))
      assertEquals(1000.0, after.durationMs.toDouble(), 100.0)
      val stored = quadrants(after.frame)
      val wanted = quadrants(expected)
      assertTrue("stored $stored shown $wanted", stored.zip(wanted).all { (a, b) -> a.zip(b).all { (x, y) -> kotlin.math.abs(x - y) < 48 } })
    } finally {
      file.delete()
    }
  }

  @Test
  fun inv_capture_29_anUprightRecordingIsLeftAsItIs() = runBlocking {
    val file = copy("video-1080-sdr.mp4")
    try {
      val before = file.readBytes()

      uprightVideoAt(context, file)

      assertArrayEquals(before, file.readBytes())
    } finally {
      file.delete()
    }
  }
}

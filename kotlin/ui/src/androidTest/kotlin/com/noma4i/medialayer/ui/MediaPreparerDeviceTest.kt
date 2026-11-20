package com.noma4i.medialayer.ui

import android.graphics.BitmapFactory
import android.media.ExifInterface
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.CannotDecode
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaPrepareIntent
import com.noma4i.medialayer.MediaPrepareJob
import com.noma4i.medialayer.MediaPrepareOutcome
import com.noma4i.medialayer.MediaProbeOutcome
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.defaultVideoLimits
import com.noma4i.medialayer.mediaPrepareDescriptor
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** The public MediaPreparer (probe, run, cancel) on a real phone: real MediaCodec and Transformer, real files, descriptors of `mediaPrepareDescriptor`. */
@RunWith(AndroidJUnit4::class)
class MediaPreparerDeviceTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context = instrumentation.targetContext
  private val preparer = MediaPreparer(context)

  private class Result(val mime: String, val width: Int, val height: Int, val frames: Int, val hasAudio: Boolean)

  private class Size(val width: Int, val height: Int) {
    override fun toString() = "${width}x$height"
  }

  /** `plain` is the source as the platform reports it turned upright, `out` is the planned and the written result. A cell that `mayFail` may also end in CannotDecode or Failed with a reason. */
  private class Cell(val name: String, val asset: String, val descriptor: ByteArray, val out: Size, val plain: Size? = null, val mayFail: Boolean = false, val check: (File, File) -> String)

  private fun video(longSide: Int, mirror: Boolean = false, policy: CannotDecode = CannotDecode.FAIL, recodeAbove: ULong? = null) =
    mediaPrepareDescriptor(MediaPrepareIntent.Video(defaultVideoLimits().copy(maxLongSide = longSide.toUInt(), recodeAboveBytes = recodeAbove), mirror, policy))

  private fun image(format: ImageFormat, edit: PhotoEdit = PhotoEdit(null, false, null, null, false, 0u)) =
    mediaPrepareDescriptor(MediaPrepareIntent.Image(edit, 0.9, format))

  private fun copyAsset(asset: String): File =
    File(context.cacheDir, asset).also { file -> instrumentation.context.assets.open(asset).use { input -> file.outputStream().use { input.copyTo(it) } } }

  private fun read(file: File): Result {
    val extractor = MediaExtractor()
    val retriever = MediaMetadataRetriever()
    try {
      extractor.setDataSource(file.absolutePath)
      retriever.setDataSource(file.absolutePath)
      val formats = (0 until extractor.trackCount).map(extractor::getTrackFormat)
      val video = formats.first { it.getString(MediaFormat.KEY_MIME).orEmpty().startsWith("video/") }
      val turned = (if (video.containsKey(MediaFormat.KEY_ROTATION)) video.getInteger(MediaFormat.KEY_ROTATION) else 0) % 180 != 0
      val width = video.getInteger(MediaFormat.KEY_WIDTH)
      val height = video.getInteger(MediaFormat.KEY_HEIGHT)
      return Result(
        video.getString(MediaFormat.KEY_MIME).orEmpty(),
        if (turned) height else width,
        if (turned) width else height,
        retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)?.toIntOrNull() ?: 0,
        formats.any { it.getString(MediaFormat.KEY_MIME).orEmpty().startsWith("audio/") }
      )
    } finally {
      extractor.release()
      retriever.release()
    }
  }

  private fun dateOf(file: File): String? {
    val retriever = MediaMetadataRetriever()
    try {
      retriever.setDataSource(file.absolutePath)
      return retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DATE)
    } finally {
      retriever.release()
    }
  }

  private fun videoRow(width: Int, height: Int, mime: String? = "video/avc", audio: Boolean? = true, frames: IntRange = 1..40): (File, File) -> String = { file, source ->
    val result = read(file)
    if (mime != null) assertEquals("codec", mime, result.mime)
    assertEquals("size", "${width}x$height", "${result.width}x${result.height}")
    if (audio != null) assertEquals("audio", audio, result.hasAudio)
    assertTrue("frames ${result.frames}", result.frames in frames)
    val found = readPrepareMetadataKeys(context, file)
    assertFalse("location kept: $found", "location" in found)
    val sourceDate = dateOf(source)
    assertTrue("source creation time kept: $sourceDate", sourceDate == null || sourceDate != dateOf(file))
    "${result.mime} ${result.width}x${result.height} frames=${result.frames} audio=${result.hasAudio} no-location"
  }

  private fun imageRow(width: Int, height: Int, mime: String, alpha: Boolean = false): (File, File) -> String = { file, _ ->
    val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeFile(file.path, it) }
    assertEquals("format", mime, options.outMimeType)
    assertEquals("size", "${width}x$height", "${options.outWidth}x${options.outHeight}")
    val bitmap = BitmapFactory.decodeFile(file.path)
    assertEquals("alpha", alpha, bitmap.hasAlpha() && (0 until bitmap.width).any { x -> (0 until bitmap.height).any { y -> bitmap.getPixel(x, y) ushr 24 != 255 } })
    bitmap.recycle()
    if (mime == "image/jpeg") {
      val exif = ExifInterface(file.path)
      assertTrue("gps kept", !exif.getLatLong(FloatArray(2)))
      assertTrue("device kept", exif.getAttribute(ExifInterface.TAG_MAKE) == null && exif.getAttribute(ExifInterface.TAG_MODEL) == null)
      assertTrue("orientation kept", exif.getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL) !in 2..8)
    }
    "$mime ${options.outWidth}x${options.outHeight} alpha=$alpha no-gps no-device"
  }

  private fun crop(x: Double, y: Double, width: Double, height: Double) = PhotoEdit(null, false, CropFractions(x, y, width, height), null, false, 0u)

  private val cells = listOf(
    Cell("H.264+AAC in limits with location (repack by the threshold)", "h264-fit-location.mp4", video(640, recodeAbove = 100_000_000u), Size(640, 360), Size(640, 360), check = videoRow(640, 360)),
    Cell("H.264 above limits", "h264-large.mp4", video(640), Size(640, 360), Size(1280, 720), check = videoRow(640, 360)),
    Cell("HEVC", "hevc.mp4", video(640), Size(640, 360), Size(640, 360), mayFail = true, check = videoRow(640, 360)),
    Cell("portrait with turn 270", "h264-turned.mp4", video(640), Size(360, 640), Size(360, 640), check = videoRow(360, 640)),
    Cell("no audio track", "h264-silent.mp4", video(320), Size(320, 180), Size(640, 360), check = videoRow(320, 180, audio = false)),
    Cell("mirror", "h264-fit-location.mp4", video(640, mirror = true), Size(640, 360), Size(640, 360), check = videoRow(640, 360)),
    Cell("4K source", "h264-4k.mp4", video(1920), Size(1920, 1080), Size(3840, 2160), mayFail = true, check = videoRow(1920, 1080)),
    Cell("4K SDR device clip", "arg:clip4kSdr", video(1920), Size(1920, 1080), mayFail = true, check = videoRow(1920, 1080, frames = 1..100000)),
    Cell("4K HDR10 device clip", "arg:clip4kHdr", video(1920), Size(1920, 1080), mayFail = true, check = videoRow(1920, 1080, frames = 1..100000)),
    Cell("1080p device clip", "arg:clip1080", video(1920), Size(1920, 1080), check = videoRow(1920, 1080, audio = null, frames = 1..100000)),
    Cell("JPEG", "photo.jpg", image(ImageFormat.JPEG), Size(800, 600), Size(800, 600), check = imageRow(800, 600, "image/jpeg")),
    Cell("JPEG cropped to the left half", "photo.jpg", image(ImageFormat.JPEG, crop(0.0, 0.0, 0.5, 1.0)), Size(400, 600), Size(800, 600), check = imageRow(400, 600, "image/jpeg")),
    Cell("JPEG turned by 90", "photo.jpg", image(ImageFormat.JPEG, PhotoEdit(null, false, null, null, false, 90u)), Size(600, 800), Size(800, 600), check = imageRow(600, 800, "image/jpeg")),
    Cell("HEIC to JPEG", "photo.heic", image(ImageFormat.JPEG), Size(800, 600), mayFail = true, check = imageRow(800, 600, "image/jpeg")),
    Cell("PNG with alpha", "alpha.png", image(ImageFormat.PNG), Size(200, 100), Size(200, 100), check = imageRow(200, 100, "image/png", alpha = true)),
    Cell("EXIF turn 6 with GPS and device tags", "exif-turned.jpg", image(ImageFormat.JPEG), Size(200, 400), Size(200, 400), check = imageRow(200, 400, "image/jpeg"))
  )

  private fun sizeOf(outcome: MediaProbeOutcome) = (outcome as? MediaProbeOutcome.File)?.let { "${it.width}x${it.height}" }

  /** Every source of the matrix gives a file by the descriptor or a failure with a reason and no file; the probe says the planned size before the run. */
  @Test
  fun theSourceMatrixGivesAFileByTheDescriptorOrAFailureWithAReason() = runBlocking<Unit> {
    val rows = mutableListOf<String>()
    val broken = mutableListOf<String>()
    val arguments = InstrumentationRegistry.getArguments()
    cells.forEachIndexed { index, cell ->
      val external = cell.asset.startsWith("arg:")
      val clip = if (external) arguments.getString(cell.asset.removePrefix("arg:")) else null
      if (external && clip == null) {
        Log.i("ML_PREPARE_MATRIX", "${cell.name} -> SKIPPED no runner argument")
        return@forEachIndexed
      }
      val source = if (clip != null) File(clip) else copyAsset(cell.asset)
      val destination = File(context.cacheDir, "prepared-$index")
      val planned = preparer.probe(source.path, cell.descriptor, null)
      val plain = preparer.probe(source.path, null, null)
      val probed = if (planned is MediaProbeOutcome.Failed) {
        if (!cell.mayFail) broken += cell.name
        "probe failed ${planned.message}"
      } else {
        if (sizeOf(planned) != cell.out.toString()) broken += "${cell.name} (planned ${sizeOf(planned)}, expected ${cell.out})"
        if (cell.plain != null && sizeOf(plain) != cell.plain.toString()) broken += "${cell.name} (plain ${sizeOf(plain)}, expected ${cell.plain})"
        "probe planned ${sizeOf(planned)} plain ${sizeOf(plain)}"
      }
      val outcome = preparer.run(MediaPrepareJob(index.toULong(), source.path, destination.path, cell.descriptor), {})
      val row = when (outcome) {
        is MediaPrepareOutcome.Done -> try {
          "${cell.name} -> $probed | Done ${outcome.mime}: ${cell.check(destination, source)}"
        } catch (error: AssertionError) {
          broken += cell.name
          "${cell.name} -> $probed | WRONG ${error.message}"
        }
        is MediaPrepareOutcome.Unchanged -> "${cell.name} -> $probed | Unchanged ${outcome.mime}".also { broken += cell.name }
        is MediaPrepareOutcome.CannotDecode -> "${cell.name} -> $probed | CannotDecode ${outcome.message}".also { if (!cell.mayFail || outcome.message.isBlank()) broken += cell.name }
        is MediaPrepareOutcome.Failed -> "${cell.name} -> $probed | Failed ${outcome.message}".also { if (!cell.mayFail || outcome.message.isBlank()) broken += cell.name }
        MediaPrepareOutcome.Cancelled -> "${cell.name} -> $probed | Cancelled".also { broken += cell.name }
      }
      if (outcome !is MediaPrepareOutcome.Done) assertFalse("a run without a file leaves none: ${cell.name}", destination.exists())
      Log.i("ML_PREPARE_MATRIX", row)
      rows += row
      destination.delete()
      if (!external) source.delete()
    }
    assertTrue("cells outside the expectation: $broken\n${rows.joinToString("\n")}", broken.isEmpty())
  }

  /** A repacked file carries neither location nor creation time; a file that is already clean and fits is passed as it is with nothing written. */
  @Test
  fun aRepackedClipHasNoTagsAndACleanOneIsPassed() = runBlocking<Unit> {
    val source = copyAsset("h264-fit-location.mp4")
    val repacked = File(context.cacheDir, "repacked.mp4")
    val untouched = File(context.cacheDir, "untouched.mp4")
    try {
      assertTrue("the fixture carries the location", "location" in readPrepareMetadataKeys(context, source))
      val descriptor = video(640, recodeAbove = 100_000_000u)

      val done = preparer.run(MediaPrepareJob(1u, source.path, repacked.path, descriptor), {})
      assertTrue(done.toString(), done is MediaPrepareOutcome.Done)
      assertFalse("location kept", "location" in readPrepareMetadataKeys(context, repacked))

      val passed = preparer.run(MediaPrepareJob(2u, repacked.path, untouched.path, descriptor), {})
      assertTrue(passed.toString(), passed is MediaPrepareOutcome.Unchanged)
      assertFalse(untouched.exists())
    } finally {
      source.delete()
      repacked.delete()
      untouched.delete()
    }
  }

  /** A device with no decoder of the source: Fail is CannotDecode with a reason and no file, UploadSource is Unchanged. */
  @Test
  fun aSourceNoDecoderReadsIsRefusedOrPassedByThePolicy() = runBlocking<Unit> {
    val source = copyAsset("h264-large.mp4")
    val destination = File(context.cacheDir, "undecodable.mp4")
    val blind = MediaPreparer(context, ::readPrepareVideoFacts, ::readPrepareMetadataKeys, { emptyList() }, ::exportPrepared)
    try {
      val refused = blind.run(MediaPrepareJob(3u, source.path, destination.path, video(640)), {})
      assertTrue(refused.toString(), refused is MediaPrepareOutcome.CannotDecode && refused.message.isNotBlank())
      assertFalse(destination.exists())

      val passed = blind.run(MediaPrepareJob(4u, source.path, destination.path, video(640, policy = CannotDecode.UploadSource)), {})
      assertTrue(passed.toString(), passed is MediaPrepareOutcome.Unchanged)
      assertFalse(destination.exists())
    } finally {
      source.delete()
    }
  }

  /** A file of another kind than the descriptor gives a failure with a reason and no file. */
  @Test
  fun aFileOfAnotherKindGivesAFailureWithAReasonAndNoFile() = runBlocking<Unit> {
    val photo = copyAsset("photo.jpg")
    val clip = copyAsset("h264-silent.mp4")
    val destination = File(context.cacheDir, "other-kind")
    try {
      val image = preparer.probe(photo.path, video(640), null)
      val movie = preparer.probe(clip.path, image(ImageFormat.JPEG), null)
      assertTrue("image probed as video: $image", image is MediaProbeOutcome.Failed && image.message.isNotBlank())
      assertTrue("video probed as image: $movie", movie is MediaProbeOutcome.Failed && movie.message.isNotBlank())

      val outcome = preparer.run(MediaPrepareJob(5u, photo.path, destination.path, video(640)), {})
      assertTrue(outcome.toString(), outcome is MediaPrepareOutcome.Failed || outcome is MediaPrepareOutcome.CannotDecode)
      assertFalse(destination.exists())
    } finally {
      photo.delete()
      clip.delete()
    }
  }

  /** `cancel` stops a running export, the run returns Cancelled and no partial file stays. */
  @Test
  fun cancelStopsTheRunAndRemovesThePartialFile() = runBlocking<Unit> {
    val source = copyAsset("h264-long.mp4")
    val destination = File(context.cacheDir, "cancelled.mp4")
    val started = AtomicBoolean(false)
    val outcome = try {
      preparer.run(MediaPrepareJob(7u, source.path, destination.path, video(1280, mirror = true)), {
        if (started.compareAndSet(false, true)) preparer.cancel(7u)
      })
    } finally {
      source.delete()
    }
    assertTrue("the export had to report before the cancel", started.get())
    assertEquals(MediaPrepareOutcome.Cancelled, outcome)
    assertFalse(destination.exists())
    assertEquals(emptyList<String>(), context.cacheDir.list()!!.filter { it.contains("cancelled.mp4") })
  }
}

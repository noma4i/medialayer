package com.noma4i.medialayer.ui

import android.app.Application
import android.app.ActivityManager
import android.content.ContentProvider
import android.content.ContentValues
import android.content.ContextWrapper
import android.content.pm.ProviderInfo
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import androidx.test.core.app.ApplicationProvider
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import com.noma4i.medialayer.CannotDecode
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaPrepareIntent
import com.noma4i.medialayer.MediaPrepareJob
import com.noma4i.medialayer.MediaPrepareOutcome
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.mediaPrepareDescriptor
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaException
import java.io.File
import java.io.FileNotFoundException
import java.io.FilterInputStream
import java.io.FilterOutputStream
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowContentResolver

class PickedContent : ContentProvider() {
  val files = mutableMapOf<String, File>()
  override fun onCreate() = true
  override fun getType(uri: Uri) = if (uri.lastPathSegment == "photo") "image/jpeg" else "video/mp4"
  override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?) =
    MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)).apply {
      addRow(arrayOf(if (uri.lastPathSegment == "photo") "photo.JPEG" else "clip"))
    }
  override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor =
    ParcelFileDescriptor.open(files[uri.lastPathSegment] ?: throw FileNotFoundException("no picked file"), ParcelFileDescriptor.MODE_READ_ONLY)
  override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
  override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = throw UnsupportedOperationException()
  override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = throw UnsupportedOperationException()
}

@RunWith(MediaLayerTestRunner::class)
class ImportMediaTest {
  private val app = ApplicationProvider.getApplicationContext<Application>()
  private val directory = java.nio.file.Files.createTempDirectory("import-media").toFile()
  private val context = object : ContextWrapper(app) { override fun getCacheDir() = directory }
  private val picked = PickedContent()

  @Before
  fun register() {
    picked.attachInfo(app, ProviderInfo().apply { authority = "picked" })
    ShadowContentResolver.registerProviderInternal("picked", picked)
  }

  @After
  fun restore() {
    directory.deleteRecursively()
    picked.files.values.forEach { it.delete() }
  }

  private fun uri(name: String) = Uri.parse("content://picked/$name")

  private fun hash(file: File): String {
    val digest = MessageDigest.getInstance("SHA-256")
    file.inputStream().use { input ->
      val buffer = ByteArray(64 * 1024)
      while (true) {
        val count = input.read(buffer)
        if (count < 0) break
        digest.update(buffer, 0, count)
      }
    }
    return digest.digest().joinToString("") { "%02x".format(it) }
  }

  @Test
  fun inv_import_01_contentImageAndVideoAreAcceptedByPreparation() {
    val manager = app.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
    val owner = object : LifecycleOwner { override val lifecycle = LifecycleRegistry.createUnsafe(this) }
    val media = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()
    try {
      runBlocking {
        val photo = File.createTempFile("picked-photo", ".jpg").apply { writeBytes(exifJpeg(40, 20, 6)) }
        val clip = File.createTempFile("picked-video", ".mp4").apply {
          app.resources.openRawResource(R.raw.medialayer_test_clip).use { input -> outputStream().use { input.copyTo(it) } }
        }
        picked.files.putAll(mapOf("photo" to photo, "video" to clip))
        val image = media.importMedia(uri("photo"))
        val video = media.importMedia(uri("video"))
        try {
          assertTrue(image.isNew && video.isNew)
          assertEquals("jpeg", image.file.extension)
          assertEquals("mp4", video.file.extension)
          assertEquals(hash(photo), hash(image.file))
          assertEquals(hash(clip), hash(video.file))
          val photoJob = MediaPrepareJob(1u, image.file.path, File(directory, "photo-out.jpg").path, mediaPrepareDescriptor(MediaPrepareIntent.Image(PhotoEdit(null, false, null, null, false, 0u), null, ImageFormat.JPEG)))
          val photoOutcome = MediaPreparer(context).run(photoJob) {}
          assertTrue(photoOutcome.toString(), photoOutcome is MediaPrepareOutcome.Done || photoOutcome is MediaPrepareOutcome.Unchanged)
          val seen = mutableListOf<File>()
          val clipJob = MediaPrepareJob(2u, video.file.path, File(directory, "clip-out.mp4").path, mediaPrepareDescriptor(MediaPrepareIntent.Video(null, false, CannotDecode.FAIL)))
          val clipPreparer = MediaPreparer(context, { file -> seen += file; VideoFacts(1280, 720, 0, 30f, 1_500_000, 2, true) }, { _, _ -> emptyList() }, { emptyList() }, { error("no export expected") })
          assertTrue(clipPreparer.run(clipJob) {} is MediaPrepareOutcome.Unchanged)
          assertEquals(listOf(video.file), seen)
          assertTrue(image.file.exists() && video.file.exists())
        } finally {
          image.file.delete()
          video.file.delete()
        }
      }
    } finally {
      media.close()
    }
  }

  @Test
  fun inv_import_02_largeVideoPreservesSha256() = runBlocking {
    val source = File.createTempFile("large-picked", ".mp4")
    picked.files["large"] = source
    source.outputStream().use { out ->
      val chunk = ByteArray(64 * 1024) { (it % 251).toByte() }
      repeat(256) { out.write(chunk) }
    }
    val imported = importMediaAt(context, uri("large"))
    assertEquals(hash(source), hash(imported.file))
    assertEquals(source.length(), imported.file.length())
  }

  @Test
  fun inv_import_02_aProviderWithoutADescriptorIsCopiedByStream() = runBlocking {
    val bytes = ByteArray(300 * 1024) { (it % 253).toByte() }
    shadowOf(app.contentResolver).registerInputStream(uri("stream"), bytes.inputStream())
    val imported = importMediaAt(context, uri("stream"))
    assertEquals(bytes.size.toLong(), imported.file.length())
    assertTrue(bytes.contentEquals(imported.file.readBytes()))
  }

  @Test
  fun inv_import_03_openReadAndWriteErrorsLeaveNoSuccessfulFile() {
    assertThrows(MediaException.Fetch::class.java) { runBlocking { importMediaAt(context, uri("missing")) } }
    val closed = AtomicInteger()
    shadowOf(app.contentResolver).registerInputStream(uri("broken"), object : InputStream() {
      var reads = 0
      override fun read(): Int = throw IOException("broken source")
      override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (reads++ > 0) throw IOException("broken source")
        buffer[offset] = 7
        return 1
      }
      override fun close() { closed.incrementAndGet() }
    })
    assertThrows(MediaException.Fetch::class.java) { runBlocking { importMediaAt(context, uri("broken")) } }
    assertEquals(1, closed.get())
    assertTrue(directory.listFiles()!!.isEmpty())
    val notDirectory = File(directory, "file").apply { writeText("occupied") }
    val badDestination = object : ContextWrapper(context) { override fun getCacheDir() = notDirectory }
    shadowOf(app.contentResolver).registerInputStream(uri("write"), byteArrayOf(1).inputStream())
    assertThrows(MediaException.Fetch::class.java) { runBlocking { importMediaAt(badDestination, uri("write")) } }
    assertEquals(listOf(notDirectory), directory.listFiles()!!.toList())
    assertEquals("occupied", notDirectory.readText())
    notDirectory.delete()
    val outputClosed = AtomicInteger()
    shadowOf(app.contentResolver).registerInputStream(uri("full"), byteArrayOf(1, 2, 3).inputStream())
    assertThrows(MediaException.Fetch::class.java) {
      runBlocking {
        importMediaAt(context, uri("full")) { file ->
          object : FilterOutputStream(file.outputStream()) {
            override fun write(buffer: ByteArray, offset: Int, length: Int) {
              out.write(buffer, offset, 1)
              throw IOException("destination full")
            }
            override fun close() { outputClosed.incrementAndGet(); super.close() }
          }
        }
      }
    }
    assertEquals(1, outputClosed.get())
    assertTrue(directory.listFiles()!!.isEmpty())
  }

  @Test
  fun inv_import_04_cancellationClosesTheStreamAndDeletesOnlyThePartialFile() = runBlocking {
    withTimeout(10_000) {
      shadowOf(app.contentResolver).registerInputStream(uri("finished"), byteArrayOf(1, 2, 3).inputStream())
      val finished = importMediaAt(context, uri("finished"))
      val reads = AtomicInteger()
      val closed = AtomicInteger()
      lateinit var job: Job
      shadowOf(app.contentResolver).registerInputStream(uri("cancelled"), object : InputStream() {
        override fun read(): Int = error("chunked reads required")
        override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
          val count = reads.incrementAndGet()
          buffer[offset] = 9
          if (count == 2) job.cancel()
          return 1
        }
        override fun close() { closed.incrementAndGet() }
      })
      job = launch(start = CoroutineStart.LAZY) { importMediaAt(context, uri("cancelled")) }
      job.start()
      job.join()
      assertTrue(job.isCancelled)
      assertEquals(2, reads.get())
      assertEquals(1, closed.get())
      assertEquals(listOf(finished.file), directory.listFiles()!!.toList())
      assertEquals(listOf<Byte>(1, 2, 3), finished.file.readBytes().toList())
      assertTrue(finished.file.delete())
      assertFalse(finished.file.exists())
    }
  }
}

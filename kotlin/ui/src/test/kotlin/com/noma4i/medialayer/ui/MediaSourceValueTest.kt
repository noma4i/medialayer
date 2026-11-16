package com.noma4i.medialayer.ui

import android.app.Application
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSourceValue
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PlaceInput
import com.noma4i.medialayer.StateListener
import com.noma4i.medialayer.toItem
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.MethodVisitor
import org.objectweb.asm.Opcodes
import org.robolectric.Shadows.shadowOf
import android.app.ActivityManager
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaSourceValueTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()

  @Test
  fun inv_source_01_the_value_keeps_the_text_as_declared_and_the_core_cleans_it_at_intake() {
    val value = MediaSourceValue("  https://o.test/a.jpg\n", Kind.IMAGE, cover = "  ", blurHash = " LEHV6nWB ")

    assertEquals("  https://o.test/a.jpg\n", value.url)
    assertEquals("  ", value.cover)
    assertEquals(" LEHV6nWB ", value.blurHash)
    assertEquals(CacheStrategy.WINDOW, value.strategy)
  }

  @Test
  fun inv_source_02_a_path_a_file_and_a_uri_build_one_file_source() {
    val fromPath = MediaSourceValue.file("/tmp/a b.mp4", Kind.VIDEO)

    assertEquals("/tmp/a b.mp4", fromPath.url)
    assertEquals(fromPath, MediaSourceValue.file(File("/tmp/a b.mp4"), Kind.VIDEO))
    assertEquals("file:///tmp/a%20b.mp4", MediaSourceValue.file(Uri.parse("file:///tmp/a%20b.mp4"), Kind.VIDEO).url)
  }

  @Test
  fun inv_source_03_the_remote_factory_keeps_the_text_and_its_headers() {
    assertEquals(" https://o.test/v.mp4 ", MediaSourceValue.remote(" https://o.test/v.mp4 ", Kind.VIDEO).url)
    assertEquals("ftp://o.test/v.mp4", MediaSourceValue.remote("ftp://o.test/v.mp4", Kind.VIDEO).url)
    assertEquals(mapOf("X-Token" to "a"), MediaSourceValue.remote("https://o.test/v.mp4", Kind.VIDEO, headers = mapOf("X-Token" to "a")).headers)
  }

  @Test
  fun inv_source_08_building_a_source_calls_the_bridge_zero_times() {
    val bridge = "com/noma4i/medialayer/MedialayerKt"
    val builders = setOf("<init>", "file", "remote", "file\$default", "remote\$default")
    val calls = mutableListOf<String>()
    listOf(MediaSourceValue::class.java, Class.forName("com.noma4i.medialayer.MediaSourceValue\$Companion")).forEach { type ->
      type.getResourceAsStream("/" + type.name.replace('.', '/') + ".class")!!.use { bytes ->
        ClassReader(bytes).accept(object : ClassVisitor(Opcodes.ASM9) {
          override fun visitMethod(access: Int, name: String, descriptor: String?, signature: String?, exceptions: Array<out String>?): MethodVisitor? {
            if (name !in builders) return null
            return object : MethodVisitor(Opcodes.ASM9) {
              override fun visitMethodInsn(opcode: Int, owner: String, method: String, descriptor: String?, isInterface: Boolean) {
                if (owner == bridge) calls += "${type.simpleName}.$name -> $method"
              }
            }
          }
        }, ClassReader.SKIP_FRAMES)
      }
    }

    assertEquals(emptyList<String>(), calls)
  }

  @Test
  fun inv_source_09_a_text_that_is_no_source_url_reaches_its_place_as_failed() {
    val manager = context.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
    val owner = object : LifecycleOwner { override val lifecycle = LifecycleRegistry.createUnsafe(this) }
    val media = openMediaLayer(context, settings = SharedPreferencesSettings(context), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()

    listOf(MediaSourceValue.remote("not a url", Kind.IMAGE), MediaSourceValue.remote("ftp://o.test/a.jpg", Kind.IMAGE), MediaSourceValue.file("clips/a.jpg", Kind.IMAGE)).forEach { source ->
      val failed = CountDownLatch(1)
      val details = CopyOnWriteArrayList<MediaState>()
      val subscription = media.subscribeState(source.url, object : StateListener {
        override fun changed(state: MediaState) { details += state; if (state == MediaState.Failed("the source has no url")) failed.countDown() }
        override fun failed(error: MediaException) = Unit
      })
      val place = media.openPlace(source.toItem())
      place.update(PlaceInput(true, false, 0uL, null, false, null, null, false, null))

      assertTrue("${source.url}: $details", failed.await(5, TimeUnit.SECONDS))
      place.release()
      subscription.close()
    }
    media.close()
  }

  @Test
  fun inv_source_04_a_bundled_image_keeps_its_resource_url_and_kind() {
    val drawable = MediaSourceValue.raw(context, R.drawable.medialayer_test_image, Kind.IMAGE)
    val asset = MediaSourceValue.asset("/art/a.webp", Kind.IMAGE)

    assertEquals("android.resource://${context.packageName}/${R.drawable.medialayer_test_image}", drawable.url)
    assertEquals(Kind.IMAGE, drawable.kind)
    assertEquals("asset:///art/a.webp", asset.url)
    assertEquals(Kind.IMAGE, asset.kind)
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

package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.os.Looper
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaImageHardwareCopyTest {
  @Test
  fun inv_android_10_hardware_copy_runs_off_main() {
    val hardware = Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888).copy(Bitmap.Config.HARDWARE, false)
    assertEquals(Bitmap.Config.HARDWARE, hardware.config)
    assertEquals(true, Looper.getMainLooper().isCurrentThread)
    val copiedOnMain = AtomicReference<Boolean>()

    val result = runBlocking {
      hardware.softwareCopy { source ->
        copiedOnMain.set(Looper.getMainLooper().isCurrentThread)
        source.copy(Bitmap.Config.ARGB_8888, false)
      }
    }

    assertEquals(false, copiedOnMain.get())
    assertEquals(Bitmap.Config.ARGB_8888, result?.config)
  }

  @Test
  fun a_software_bitmap_is_returned_without_a_copy() {
    val software = Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888)

    val result = runBlocking { software.softwareCopy { error("a software bitmap needs no copy") } }

    assertSame(software, result)
  }
}

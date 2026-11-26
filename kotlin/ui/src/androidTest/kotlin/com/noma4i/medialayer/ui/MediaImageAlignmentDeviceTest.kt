package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSourceValue
import java.io.File
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** A MediaImage with TopCenter and a red tint draws the picture at the top of a tall frame in red, on a real phone. */
@RunWith(AndroidJUnit4::class)
class MediaImageAlignmentDeviceTest {
  @get:Rule val rule = createComposeRule()

  @Test
  fun inv_image_120_alignmentAndColorFilterReachTheDrawnImage() {
    val context = InstrumentationRegistry.getInstrumentation().targetContext
    val file = File(context.cacheDir, "align-white.png")
    file.outputStream().use { out ->
      Bitmap.createBitmap(40, 40, Bitmap.Config.ARGB_8888).apply { eraseColor(AndroidColor.WHITE) }.compress(Bitmap.CompressFormat.PNG, 100, out)
    }
    val source = MediaSourceValue.file(file, Kind.IMAGE)
    val opened = MediaLayer.open(context)
    rule.setContent {
      MediaLayerProvider(opened) {
        Box(Modifier.size(40.dp, 120.dp).testTag("frame")) {
          MediaImage(source, Modifier.size(40.dp, 120.dp), contentScale = ContentScale.Fit, alignment = Alignment.TopCenter, colorFilter = ColorFilter.tint(Color.Red, BlendMode.SrcIn))
        }
      }
    }
    fun frame() = rule.onNodeWithTag("frame").captureToImage().toPixelMap()
    rule.waitUntil(30_000) { frame().let { it[it.width / 2, 2] == Color.Red } }
    val pixels = frame()
    assertNotEquals(Color.Red, pixels[pixels.width / 2, pixels.height - 3])
  }
}

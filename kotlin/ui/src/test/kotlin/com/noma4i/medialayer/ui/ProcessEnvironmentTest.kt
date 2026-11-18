package com.noma4i.medialayer.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSource
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private data class Pic(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class ProcessEnvironmentTest {
  @get:Rule val rule = createComposeRule()

  @Test
  fun inv_api_71_the_process_environment_outlives_the_screen_that_installed_it() {
    val layer = RecordingLayer()
    val opened = CompletableDeferred(layer)
    var provided by mutableStateOf(true)
    rule.setContent { if (provided) MediaLayerProvider(opened) { } }
    rule.waitForIdle()
    runBlocking { Media.retry(Pic("https://o.test/a.png")) }

    provided = false
    rule.waitForIdle()
    val between = runCatching { runBlocking { Media.retry(Pic("https://o.test/b.png")) } }
    assertEquals("the window between two compositions is not Closed", null, between.exceptionOrNull())

    provided = true
    rule.waitForIdle()
    runBlocking { Media.retry(Pic("https://o.test/c.png")) }

    layer.destroy()
    val closed = runCatching { runBlocking { Media.retry(Pic("https://o.test/d.png")) } }.exceptionOrNull()
    assertTrue("a closed layer stays Closed: $closed", closed is MediaException.Closed)
  }
}

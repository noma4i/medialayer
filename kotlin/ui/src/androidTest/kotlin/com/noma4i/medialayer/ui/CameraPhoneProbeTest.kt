package com.noma4i.medialayer.ui

import android.Manifest
import android.media.MediaExtractor
import android.media.MediaFormat
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CaptureKind
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.captureProfile
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Phone probe: the real camera, the real microphone, the public `MediaCamera` the app uses, no fakes and no gate on the stream.
 * Every assertion is a fact measured on the device: the time from the stop call to the delivered clip, and the file the camera wrote.
 * Output: logcat tag `MediaLayerProbe`, lines `PROBE <name> key=value`.
 */
@RunWith(AndroidJUnit4::class)
class CameraPhoneProbeTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context = instrumentation.targetContext
  private val files = ArrayList<File>()
  private var layer: MediaLayer? = null
  private var scenario: ActivityScenario<ComponentActivity>? = null

  @Before
  fun open() = runBlocking {
    listOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO).forEach { instrumentation.uiAutomation.grantRuntimePermission(context.packageName, it) }
    val memory = android.app.ActivityManager.MemoryInfo().also { (context.getSystemService(android.content.Context.ACTIVITY_SERVICE) as android.app.ActivityManager).getMemoryInfo(it) }
    val manager = context.getSystemService(android.content.Context.ACTIVITY_SERVICE) as android.app.ActivityManager
    probe("device", "totalMb" to memory.totalMem / MB, "heapClassMb" to manager.memoryClass, "lowRam" to manager.isLowRamDevice, "availMb" to memory.availMem / MB)
    layer = withTimeout(TIMEOUT_MS) { MediaLayer.open(context).await() }
  }

  @After
  fun release() {
    scenario?.close()
    layer?.close()
    files.forEach { it.delete() }
  }

  private fun probe(name: String, vararg facts: Pair<String, Any?>) {
    val line = "PROBE $name " + facts.joinToString(" ") { "${it.first}=${it.second}" }
    Log.i(TAG, line)
    println(line)
  }

  /** Per track of the file as the camera wrote it: mime, track duration in us, packet count. */
  private fun tracks(file: File): Map<String, Pair<Long, Int>> {
    val extractor = MediaExtractor()
    try {
      extractor.setDataSource(file.absolutePath)
      val formats = (0 until extractor.trackCount).map { extractor.getTrackFormat(it) }
      val count = IntArray(formats.size)
      formats.indices.forEach { extractor.selectTrack(it) }
      while (extractor.sampleTrackIndex >= 0) {
        count[extractor.sampleTrackIndex]++
        extractor.advance()
      }
      return formats.indices.associate { i ->
        val mime = formats[i].getString(MediaFormat.KEY_MIME).orEmpty()
        mime to ((if (formats[i].containsKey(MediaFormat.KEY_DURATION)) formats[i].getLong(MediaFormat.KEY_DURATION) else -1L) to count[i])
      }
    } finally {
      extractor.release()
    }
  }

  private fun take(name: String, kind: CaptureKind, lens: CameraLens, holdMs: Long) = runBlocking {
    val profile = captureProfile(kind).copy(audio = true)
    val composed = CompletableDeferred<MediaCamera>()
    val launched = ActivityScenario.launch(ComponentActivity::class.java).also { scenario = it }
    launched.onActivity { activity ->
      activity.setContentView(ComposeView(activity).apply {
        setContent {
          val made = remember { MediaCamera(context, profile, lens) }
          DisposableEffect(made) { onDispose { made.close() } }
          LaunchedEffect(made) { composed.complete(made) }
          MediaCameraPreview(made, Modifier.fillMaxSize())
        }
      })
    }
    val camera = withTimeout(TIMEOUT_MS) { composed.await() }
    val ready = withTimeout(TIMEOUT_MS) { camera.state.first { it.isReady || it.phase == CameraPhase.FAILED } }
    probe("$name-ready", "phase" to ready.phase, "fault" to ready.fault, "state" to ready)
    assertEquals("the camera is ready, last state $ready", CameraPhase.READY, ready.phase)
    delay(500)
    camera.startRecording()
    withTimeout(TIMEOUT_MS) { camera.state.first { it.isRecording && it.recordedMs >= profile.minTakeMs } }
    delay(holdMs)
    val stopping = System.nanoTime()
    val clip = camera.stopRecording()
    val stopToClipMs = (System.nanoTime() - stopping) / 1_000_000
    files.add(clip.file)
    val found = tracks(clip.file)
    val video = found.entries.firstOrNull { it.key.startsWith("video/") }?.value
    val audio = found.entries.firstOrNull { it.key.startsWith("audio/") }?.value
    probe(
      name,
      "kind" to kind, "lens" to lens, "holdMs" to holdMs, "stopToClipMs" to stopToClipMs, "clipMs" to clip.durationMs, "bytes" to clip.file.length(),
      "videoUs" to video?.first, "videoPackets" to video?.second, "audioUs" to audio?.first, "audioPackets" to audio?.second,
    )
    assertTrue("$name: stop to clip took $stopToClipMs ms, the pause must not grow with the take", stopToClipMs <= STOP_TO_CLIP_MAX_MS)
    assertTrue("$name: the file has video packets", (video?.second ?: 0) > 0)
    assertTrue("$name: the file has sound packets", (audio?.second ?: 0) > 0)
  }

  @Test
  fun a_five_second_take_with_sound_is_delivered_without_a_long_pause() = take("take5s", CaptureKind.CIRCLE, CameraLens.FRONT, 5_000)

  @Test
  fun a_fifteen_second_take_with_sound_is_delivered_without_a_long_pause() = take("take15s", CaptureKind.CIRCLE, CameraLens.FRONT, 15_000)

  private companion object {
    const val MB = 1024L * 1024L
    const val TAG = "MediaLayerProbe"
    const val TIMEOUT_MS = 30_000L
    const val STOP_TO_CLIP_MAX_MS = 1_500L
  }
}

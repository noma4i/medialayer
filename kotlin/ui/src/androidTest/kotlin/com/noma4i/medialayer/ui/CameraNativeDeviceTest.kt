package com.noma4i.medialayer.ui

import android.Manifest
import android.media.MediaExtractor
import android.media.MediaFormat
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import android.media.MediaMetadataRetriever
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
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CameraNativeDeviceTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context = instrumentation.targetContext
  private val files = ArrayList<File>()
  private val diagnostics = CopyOnWriteArrayList<File>()
  private val reported = CopyOnWriteArrayList<NativeFailure>()
  private val heard = kotlinx.coroutines.flow.MutableStateFlow(emptyList<MicrophoneAvailability>())
  private val readings = CopyOnWriteArrayList<String>()
  private var layer: MediaLayer? = null
  private var opened: Deferred<MediaLayer>? = null
  private var scenario: ActivityScenario<ComponentActivity>? = null

  @Before
  fun open() = runBlocking {
    listOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO).forEach { instrumentation.uiAutomation.grantRuntimePermission(context.packageName, it) }
    opened = MediaLayer.open(context).also { layer = withTimeout(TIMEOUT_MS) { it.await() } }
  }

  @After
  fun release() {
    scenario?.close()
    layer?.close()
    files.forEach { it.delete() }
  }

  private class Packets(val video: Int, val audio: Int, val audioMime: String?)

  private fun packets(file: File): Packets {
    val extractor = MediaExtractor()
    try {
      extractor.setDataSource(file.absolutePath)
      val mimes = (0 until extractor.trackCount).map { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME).orEmpty() }
      val counts = IntArray(mimes.size)
      mimes.indices.forEach { extractor.selectTrack(it) }
      while (extractor.sampleTrackIndex >= 0) {
        counts[extractor.sampleTrackIndex]++
        extractor.advance()
      }
      val video = mimes.indexOfFirst { it.startsWith("video/") }
      val audio = mimes.indexOfFirst { it.startsWith("audio/") }
      return Packets(if (video >= 0) counts[video] else 0, if (audio >= 0) counts[audio] else 0, mimes.getOrNull(audio))
    } finally {
      extractor.release()
    }
  }

  private fun describe(file: File): String {
    val retriever = MediaMetadataRetriever()
    val extractor = MediaExtractor()
    try {
      retriever.setDataSource(file.absolutePath)
      extractor.setDataSource(file.absolutePath)
      val formats = (0 until extractor.trackCount).map { extractor.getTrackFormat(it).toString() }
      val keys = listOf(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH, MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT, MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION, MediaMetadataRetriever.METADATA_KEY_DURATION)
      return "${file.path} ${file.length()} bytes tracks=$formats retriever=${keys.map { retriever.extractMetadata(it) }} packets=${packets(file).let { "${it.video}/${it.audio}" }}"
    } catch (error: Exception) {
      return "${file.path} ${file.length()} bytes unreadable: $error"
    } finally {
      extractor.release()
      retriever.release()
    }
  }

  private fun diagnosing(): CameraServices {
    val live = CameraServices.live(context)
    return CameraServices(live.admitSession, live.admitShot, live.audio, { file ->
      live.clipInfo(file).also { readings.add("clipInfo ${file.name} = $it") }
    }, { file ->
      diagnostics.add(File(context.cacheDir, "diagnostic-${System.nanoTime()}.mp4").also { file.copyTo(it, overwrite = true) })
      try {
        live.uprightVideo(file)
        readings.add("uprightVideo ${file.name} done, after: ${describe(file)}")
      } catch (error: Exception) {
        readings.add("uprightVideo ${file.name} threw $error, after: ${describe(file)}")
        throw error
      }
    }, live.upright, live.directory, live.audioCoverage)
  }

  private fun record(audio: Boolean) = runBlocking {
    val profile = captureProfile(CaptureKind.MOMENT).copy(audio = audio)
    val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.collect { reported.add(it) } }
    val hearing = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { checkNotNull(layer).microphoneAvailability.collect { heard.value += it } }
    val composed = CompletableDeferred<MediaCamera>()
    val launched = ActivityScenario.launch(ComponentActivity::class.java).also { scenario = it }
    launched.onActivity { activity ->
      activity.setContentView(ComposeView(activity).apply {
        setContent {
          MediaLayerProvider(checkNotNull(opened)) {
            val made = remember { MediaCamera(profile, CameraLens.BACK, CameraXBackend(context), diagnosing()) }
            DisposableEffect(made) { onDispose { made.close() } }
            LaunchedEffect(made) { composed.complete(made) }
            MediaCameraPreview(made, Modifier.fillMaxSize())
          }
        }
      })
    }
    val made = withTimeout(TIMEOUT_MS) { composed.await() }
    assertNotNull("the provider connected the layer to the process environment", ProcessMediaEnvironment.current)
    assertNotNull("the layer is connected to the audio link", Media.link)
    val ready = withTimeout(TIMEOUT_MS) { made.state.first { it.isReady || it.phase == CameraPhase.FAILED } }
    assertEquals("the camera is ready, fault ${ready.fault}, native failures=${reported.toList()}", CameraPhase.READY, ready.phase)
    if (audio) {
      try {
        withTimeout(TIMEOUT_MS) { heard.first { history -> MicrophoneAvailability.AVAILABLE in history.dropWhile { it != MicrophoneAvailability.NO_SOUND } } }
      } catch (error: kotlinx.coroutines.TimeoutCancellationException) {
        throw AssertionError("the microphone never became available; availability=${heard.value}; native failures=${reported.toList()}", error)
      }
    }
    val clips = (1..2).map {
      try {
        made.startRecording()
        withTimeout(TIMEOUT_MS) { made.state.first { it.isRecording && it.recordedMs >= profile.minTakeMs } }
        made.stopRecording().also { clip -> files.add(clip.file) }
      } catch (error: MediaException.Camera) {
        throw AssertionError("take $it of 2 failed: $error; availability=${heard.value}; native failures=${reported.toList()}; readers=${readings.toList()}; producer files before upright=${diagnostics.map(::describe)}", error)
      }
    }
    failures.cancel()
    hearing.cancel()
    assertNotEquals("two takes are two files", clips[0].file, clips[1].file)
    clips.forEach { clip ->
      val found = packets(clip.file)
      assertTrue("video packets in ${clip.file}", found.video > 0)
      assertEquals("the audio packets follow the profile", audio, found.audio > 0)
      if (audio) assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, found.audioMime)
      assertTrue("the clip is ${clip.durationMs} ms, the least take is ${profile.minTakeMs}", clip.durationMs.toULong() >= profile.minTakeMs)
    }
  }

  @Test
  fun two_takes_with_sound_are_two_mp4_files_with_video_and_aac_packets() = record(audio = true)

  @Test
  fun two_takes_without_sound_are_two_mp4_files_with_video_packets_only() = record(audio = false)

  private companion object {
    const val TIMEOUT_MS = 30_000L
  }
}

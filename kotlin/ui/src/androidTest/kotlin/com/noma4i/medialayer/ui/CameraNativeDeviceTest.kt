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

  private fun diagnosing(damage: Damage? = null): CameraServices {
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
    }, live.upright, live.directory, { file ->
      val seen = if (damage == null) file else damaged(file, damage).also { files.add(it) }
      live.audioCoverage(seen).also { readings.add("coverage ${file.name} $damage = $it timeline=${timeline(seen)}") }
    })
  }

  /** Per track: the container duration, the first and the last packet PTS, the packet count and the last packet size, read from the file as it is. */
  private fun timeline(file: File): String {
    val extractor = MediaExtractor()
    try {
      extractor.setDataSource(file.absolutePath)
      val formats = (0 until extractor.trackCount).map { extractor.getTrackFormat(it) }
      val first = LongArray(formats.size) { -1 }
      val last = LongArray(formats.size) { -1 }
      val count = IntArray(formats.size)
      formats.indices.forEach { extractor.selectTrack(it) }
      while (extractor.sampleTrackIndex >= 0) {
        val track = extractor.sampleTrackIndex
        val time = extractor.sampleTime
        if (first[track] < 0) first[track] = time
        last[track] = time
        count[track]++
        extractor.advance()
      }
      return formats.indices.joinToString("; ") { "${formats[it].getString(MediaFormat.KEY_MIME)} duration=${if (formats[it].containsKey(MediaFormat.KEY_DURATION)) formats[it].getLong(MediaFormat.KEY_DURATION) else null} first=${first[it]} last=${last[it]} packets=${count[it]}" }
    } catch (error: Exception) {
      return "unreadable $error"
    } finally {
      extractor.release()
    }
  }

  private enum class Damage { HEAD, MIDDLE, TAIL }

  private enum class Disturbance { LENS, PREVIEW }

  /** A real copy of the file with the audio packets of one part removed, so the scan reads it through the real extractor and decoder. */
  private fun damaged(file: File, damage: Damage): File {
    val output = File(context.cacheDir, "damaged-${System.nanoTime()}.mp4")
    val extractor = MediaExtractor()
    val muxer = android.media.MediaMuxer(output.absolutePath, android.media.MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
    try {
      extractor.setDataSource(file.absolutePath)
      val formats = (0 until extractor.trackCount).map { extractor.getTrackFormat(it) }
      val video = formats.indexOfFirst { it.getString(MediaFormat.KEY_MIME)!!.startsWith("video/") }
      val length = formats[video].getLong(MediaFormat.KEY_DURATION)
      val cut = when (damage) { Damage.HEAD -> 0L..length * 3 / 10; Damage.MIDDLE -> length * 4 / 10..length * 6 / 10; Damage.TAIL -> length * 7 / 10..length }
      val mapped = formats.indices.map { muxer.addTrack(formats[it]) }
      formats.indices.forEach { extractor.selectTrack(it) }
      muxer.start()
      val buffer = java.nio.ByteBuffer.allocate(1 shl 20)
      val info = android.media.MediaCodec.BufferInfo()
      while (extractor.sampleTrackIndex >= 0) {
        val track = extractor.sampleTrackIndex
        val time = extractor.sampleTime
        info.set(0, extractor.readSampleData(buffer, 0), time, extractor.sampleFlags)
        if (track == video || time !in cut) muxer.writeSampleData(mapped[track], buffer, info)
        extractor.advance()
      }
      muxer.stop()
      return output
    } finally {
      extractor.release()
      muxer.release()
    }
  }

  private fun record(audio: Boolean, damage: Damage? = null, lens: CameraLens = CameraLens.BACK, kind: CaptureKind = CaptureKind.MOMENT, mirrored: Boolean = false, disturb: Disturbance? = null) = runBlocking {
    val profile = captureProfile(kind).copy(audio = audio).let { if (mirrored) it.copy(mirrorFrontVideo = true) else it }
    val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.collect { reported.add(it) } }
    val hearing = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { checkNotNull(layer).microphoneAvailability.collect { heard.value += it } }
    val composed = CompletableDeferred<MediaCamera>()
    val launched = ActivityScenario.launch(ComponentActivity::class.java).also { scenario = it }
    launched.onActivity { activity ->
      activity.setContentView(ComposeView(activity).apply {
        setContent {
          MediaLayerProvider(checkNotNull(opened)) {
            val made = remember { MediaCamera(profile, lens, CameraXBackend(context), diagnosing(damage)) }
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
      // The preview microphone starts silently: noSound comes only from a silence the platform reports (INV-AUDIO-86), the take below starts on its first live sound.
      kotlinx.coroutines.delay(500)
      assertTrue("the microphone start blinked no sound; availability=${heard.value}; native failures=${reported.toList()}", MicrophoneAvailability.NO_SOUND !in heard.value)
    }
    if (damage != null) {
      made.startRecording()
      withTimeout(TIMEOUT_MS) { made.state.first { it.isRecording && it.recordedMs >= profile.minTakeMs } }
      val refused = try { made.stopRecording(); null } catch (error: MediaException.Camera) { error }
      failures.cancel()
      hearing.cancel()
      assertEquals("$damage audio is refused, readings=${readings.toList()}", com.noma4i.medialayer.CameraRefusal.MICROPHONE_SILENT, refused?.reason)
      assertTrue("the refused file is deleted", CaptureFiles.directory(context).listFiles().orEmpty().isEmpty())
      return@runBlocking
    }
    if (disturb != null) {
      made.startRecording()
      withTimeout(TIMEOUT_MS) { made.state.first { it.isRecording && it.recordedMs >= profile.minTakeMs } }
      when (disturb) {
        Disturbance.LENS -> made.setLens(CameraLens.FRONT)
        Disturbance.PREVIEW -> launched.onActivity { it.setContentView(android.view.View(it)) }
      }
      kotlinx.coroutines.delay(DISTURB_SETTLE_MS)
      failures.cancel()
      hearing.cancel()
      assertTrue("no GL error is blamed on the draw, native failures=${reported.toList()}", reported.none { it.message.contains("GL ") })
      try { made.stopRecording().also { files.add(it.file) } } catch (_: MediaException.Camera) {}
      return@runBlocking
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

  /** INV-AUDIO-85 on the platform: the circle stop of the Yupi A16 case, then a voice at once while the preview lives, three times; a voice during a take is busy. */
  @Test
  fun inv_audio_85_a_voice_right_after_a_circle_stop_records_while_the_preview_lives() = runBlocking {
    val profile = captureProfile(CaptureKind.CIRCLE).copy(mirrorFrontVideo = true)
    val composed = CompletableDeferred<MediaCamera>()
    val launched = ActivityScenario.launch(ComponentActivity::class.java).also { scenario = it }
    launched.onActivity { activity ->
      activity.setContentView(ComposeView(activity).apply {
        setContent {
          MediaLayerProvider(checkNotNull(opened)) {
            val made = remember { MediaCamera(profile, CameraLens.FRONT, CameraXBackend(context), diagnosing(null)) }
            DisposableEffect(made) { onDispose { made.close() } }
            LaunchedEffect(made) { composed.complete(made) }
            MediaCameraPreview(made, Modifier.fillMaxSize())
          }
        }
      })
    }
    val camera = withTimeout(TIMEOUT_MS) { composed.await() }
    assertEquals(CameraPhase.READY, withTimeout(TIMEOUT_MS) { camera.state.first { it.isReady || it.phase == CameraPhase.FAILED } }.phase)
    repeat(3) { run ->
      camera.startRecording()
      withTimeout(TIMEOUT_MS) { camera.state.first { it.isRecording && it.recordedMs >= profile.minTakeMs } }
      val busy = MediaVoiceRecorder(context, maxDurationMs = 300_000)
      val during = withTimeout(VOICE_MS) { runCatching { busy.start(File(context.cacheDir, "busy-$run.m4a")) } }.exceptionOrNull()
      assertEquals("run $run: a voice during the take: $during", com.noma4i.medialayer.VoiceRefusal.MICROPHONE_BUSY, (during as? MediaException.Voice)?.reason)
      files.add(camera.stopRecording().file)
      val file = File(context.cacheDir, "voice-$run.m4a").also { files.add(it) }
      val recorder = MediaVoiceRecorder(context, maxDurationMs = 300_000)
      val asked = System.nanoTime()
      val started = withTimeout(VOICE_MS) { runCatching { recorder.start(file) } }
      println("V2V run=$run voice start returned after ${(System.nanoTime() - asked) / 1_000_000} ms: ${started.exceptionOrNull()}")
      assertEquals("run $run: the voice right after the circle stop records", null, started.exceptionOrNull())
      kotlinx.coroutines.delay(1_500)
      val voice = recorder.stop()
      assertTrue("run $run: the voice file has audio, ${voice.durationMs} ms", file.length() > 0 && voice.durationMs > 0L)
    }
  }

  @Test
  fun inv_capture_78_a_lens_change_during_a_take_gives_no_gl_failure() = record(audio = false, disturb = Disturbance.LENS)

  @Test
  fun inv_capture_78_the_preview_leaving_during_a_take_gives_no_gl_failure() = record(audio = false, disturb = Disturbance.PREVIEW)

  @Test
  fun two_takes_with_sound_are_two_mp4_files_with_video_and_aac_packets() = record(audio = true)

  @Test
  fun two_circle_takes_from_the_front_camera_with_the_mirror_are_two_mp4_files_with_video_and_aac_packets() = record(audio = true, lens = CameraLens.FRONT, kind = CaptureKind.CIRCLE, mirrored = true)

  @Test
  fun two_takes_without_sound_are_two_mp4_files_with_video_packets_only() = record(audio = false)

  @Test
  fun audio_missing_at_the_head_is_refused_and_the_file_deleted() = record(audio = true, damage = Damage.HEAD)

  @Test
  fun audio_missing_in_the_middle_is_refused_and_the_file_deleted() = record(audio = true, damage = Damage.MIDDLE)

  @Test
  fun audio_missing_at_the_tail_is_refused_and_the_file_deleted() = record(audio = true, damage = Damage.TAIL)

  private companion object {
    const val VOICE_MS = 10_000L
    const val TIMEOUT_MS = 30_000L
    const val DISTURB_SETTLE_MS = 2_000L
  }
}

package com.noma4i.medialayer.ui

import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLExt
import android.opengl.GLES20
import android.media.AudioFormat
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaRecorder
import android.os.SystemClock
import android.view.Surface
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.NativeFailure
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CameraTakeDeviceTest {
  private val context = InstrumentationRegistry.getInstrumentation().targetContext
  private val audioGeneration = 7L
  private val base = SystemClock.elapsedRealtimeNanos()
  private var frame = 0
  private val opened = ArrayList<AutoCloseable>()

  @After
  fun release() {
    opened.asReversed().forEach { try { it.close() } catch (_: Throwable) {} }
  }

  private class Recorder : CameraTakeListener {
    val events = LinkedBlockingQueue<String>()
    val encoded = java.util.concurrent.CopyOnWriteArrayList<Long>()
    @Volatile var baseUs = Long.MAX_VALUE
    val firstEncoded = CountDownLatch(1)
    val outcomes = AtomicInteger()
    @Volatile var surface: Surface? = null
    @Volatile var failure: NativeFailure? = null

    override fun onBegan(generation: Long, inputSurface: Surface) { surface = inputSurface; events.add("began") }

    override fun onVideoEncoded(generation: Long, ptsUs: Long) { encoded.add(ptsUs); firstEncoded.countDown() }

    override fun onStarted(generation: Long, baseUs: Long) { this.baseUs = baseUs; events.add("started") }

    override fun onFinished(generation: Long, file: File, durationUs: Long) { outcomes.incrementAndGet(); events.add("finished") }

    override fun onFailed(generation: Long, failure: NativeFailure) { this.failure = failure; outcomes.incrementAndGet(); events.add("failed") }

    override fun onCancelled(generation: Long) { outcomes.incrementAndGet(); events.add("cancelled") }

    fun expect(event: String) = assertEquals(event, events.poll(TIMEOUT_SECONDS, TimeUnit.SECONDS))

    fun keptPackets() = encoded.count { it >= baseUs }
  }

  private class Packets(val video: Int, val audio: Int)

  private class Egl(target: Surface) : AutoCloseable {
    private val display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
    private var context = EGL14.EGL_NO_CONTEXT
    private var window = EGL14.EGL_NO_SURFACE

    init {
      val version = IntArray(2)
      check(EGL14.eglInitialize(display, version, 0, version, 1))
      val attributes = intArrayOf(
        EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT, EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8, EGL14.EGL_ALPHA_SIZE, 8,
        EGL_RECORDABLE_ANDROID, 1, EGL14.EGL_NONE
      )
      val configs = arrayOfNulls<EGLConfig>(1)
      val count = IntArray(1)
      check(EGL14.eglChooseConfig(display, attributes, 0, configs, 0, 1, count, 0) && count[0] > 0)
      context = EGL14.eglCreateContext(display, configs[0], EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0)
      window = EGL14.eglCreateWindowSurface(display, configs[0], target, intArrayOf(EGL14.EGL_NONE), 0)
      check(EGL14.eglMakeCurrent(display, window, window, context))
    }

    fun draw(index: Int, timestampNs: Long) {
      GLES20.glClearColor((index * 0.1f) % 1f, 0.3f, 0.6f, 1f)
      GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
      EGLExt.eglPresentationTimeANDROID(display, window, timestampNs)
      check(EGL14.eglSwapBuffers(display, window))
    }

    override fun close() {
      EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
      EGL14.eglDestroySurface(display, window)
      EGL14.eglDestroyContext(display, context)
      EGL14.eglTerminate(display)
    }
  }

  private fun file() = File(context.cacheDir, "camera-take-${System.nanoTime()}.mp4").also { it.delete() }

  private fun videoFormat() = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, 320, 240).apply {
    setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
    setInteger(MediaFormat.KEY_BIT_RATE, 500_000)
    setInteger(MediaFormat.KEY_FRAME_RATE, 30)
    setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
  }

  private fun audioFormat() = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, 1).apply {
    setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
    setInteger(MediaFormat.KEY_BIT_RATE, 64_000)
  }

  private fun settings(audio: Boolean, heldLimitBytes: Long = 16L shl 20) =
    CameraTakeSettings(videoFormat(), if (audio) audioFormat() else null, VideoTimebase.BOOTTIME, audioGeneration, heldLimitBytes, 60_000_000L, null)

  private fun take(target: File, settings: CameraTakeSettings, listener: Recorder, released: AtomicInteger = AtomicInteger(), admit: (Long) -> AutoCloseable = { AutoCloseable { released.incrementAndGet() } }) =
    CameraTake(1L, target, settings, admit, listener).also { opened.add(AutoCloseable { it.cancel() }) }

  private fun source(surface: Surface) = Egl(surface).also { opened.add(it) }

  private fun draw(egl: Egl, frames: Int) = repeat(frames) {
    egl.draw(frame, base + frame * FRAME_NS)
    frame++
  }

  private fun pcm(generation: Long, chunks: Int = 10): List<PcmChunk> {
    val spec = PcmSpec(MediaRecorder.AudioSource.MIC, SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    return List(chunks) { index -> PcmChunk(generation, spec, ByteBuffer.allocateDirect(FRAMES * 2), FRAMES, index.toLong() * FRAMES, index * FRAMES * 1_000_000L / SAMPLE_RATE) }
  }

  private fun admitLive(take: CameraTake, chunks: List<PcmChunk>) {
    take.audioTimebase(audioGeneration, base)
    chunks.forEach { take.audio(it) }
    take.confirm()
  }

  private fun packets(file: File): Packets {
    val extractor = MediaExtractor()
    try {
      extractor.setDataSource(file.absolutePath)
      val kinds = List(extractor.trackCount) { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)!! }
      repeat(extractor.trackCount) { extractor.selectTrack(it) }
      val counts = IntArray(extractor.trackCount)
      val buffer = ByteBuffer.allocate(1 shl 20)
      while (extractor.readSampleData(buffer, 0) >= 0) {
        counts[extractor.sampleTrackIndex]++
        extractor.advance()
      }
      return Packets(counts.filterIndexed { i, _ -> kinds[i].startsWith("video/") }.sum(), counts.filterIndexed { i, _ -> kinds[i].startsWith("audio/") }.sum())
    } finally {
      extractor.release()
    }
  }

  @Test
  fun livePcmAndConfirmOpenTheFileWithRealPacketsAndFinishKeepsTheLastFrames() {
    val target = file()
    val recorder = Recorder()
    val released = AtomicInteger()
    val take = take(target, settings(true), recorder, released)
    take.begin()
    recorder.expect("began")
    val egl = source(recorder.surface!!)
    draw(egl, 5)
    assertTrue(recorder.firstEncoded.await(TIMEOUT_SECONDS, TimeUnit.SECONDS))
    assertFalse("the file must not exist before the PCM and the confirmation", target.exists())
    admitLive(take, pcm(audioGeneration))
    recorder.expect("started")
    assertTrue(target.exists())
    draw(egl, 5)
    take.finish()
    recorder.expect("finished")
    val found = packets(target)
    assertTrue(found.audio > 0)
    assertTrue(found.video > 0)
    assertEquals("every video packet from the first keyframe on must be in the file", recorder.keptPackets(), found.video)
    assertFalse("the encoder input surface is released with the take", recorder.surface!!.isValid)
    assertEquals(1, released.get())
    assertEquals(1, recorder.outcomes.get())
  }

  @Test
  fun pcmOfAnotherGenerationDoesNotOpenTheFile() {
    val target = file()
    val recorder = Recorder()
    val take = take(target, settings(true), recorder)
    take.begin()
    recorder.expect("began")
    val egl = source(recorder.surface!!)
    draw(egl, 3)
    assertTrue(recorder.firstEncoded.await(TIMEOUT_SECONDS, TimeUnit.SECONDS))
    take.audioTimebase(audioGeneration + 1, base)
    pcm(audioGeneration + 1).forEach { take.audio(it) }
    take.confirm()
    take.finish()
    recorder.expect("failed")
    assertFalse(target.exists())
    assertEquals(1, recorder.outcomes.get())
  }

  @Test
  fun cancelAfterStartGivesOneOutcomeAndDeletesTheFile() {
    val target = file()
    val recorder = Recorder()
    val released = AtomicInteger()
    val take = take(target, settings(true), recorder, released)
    take.begin()
    recorder.expect("began")
    val egl = source(recorder.surface!!)
    draw(egl, 3)
    admitLive(take, pcm(audioGeneration))
    recorder.expect("started")
    take.cancel()
    recorder.expect("cancelled")
    take.cancel()
    take.finish()
    assertFalse(target.exists())
    assertEquals(1, recorder.outcomes.get())
    assertEquals(1, released.get())
    assertFalse(recorder.surface!!.isValid)
  }

  @Test
  fun refusedReserveGivesOneOutcomeWithoutAFile() {
    val target = file()
    val recorder = Recorder()
    take(target, settings(true), recorder, admit = { throw IllegalStateException("budget refused") }).begin()
    recorder.expect("failed")
    assertNotNull(recorder.failure)
    assertFalse(target.exists())
    assertEquals(1, recorder.outcomes.get())
    assertEquals(null, recorder.surface)
  }

  @Test
  fun pcmBeyondTheReserveGivesOneOutcomeWithoutAFile() {
    val target = file()
    val recorder = Recorder()
    val released = AtomicInteger()
    val take = take(target, settings(true, heldLimitBytes = FRAMES * 2L - 1), recorder, released)
    take.begin()
    recorder.expect("began")
    val egl = source(recorder.surface!!)
    draw(egl, 2)
    admitLive(take, pcm(audioGeneration, 3))
    recorder.expect("failed")
    assertFalse(target.exists())
    assertEquals(1, recorder.outcomes.get())
    assertEquals(1, released.get())
  }

  @Test
  fun withoutAudioTheTakeIsAVideoFileOnly() {
    val target = file()
    val recorder = Recorder()
    val take = take(target, settings(false), recorder)
    take.begin()
    recorder.expect("began")
    val egl = source(recorder.surface!!)
    draw(egl, 5)
    assertTrue(recorder.firstEncoded.await(TIMEOUT_SECONDS, TimeUnit.SECONDS))
    assertFalse(target.exists())
    take.confirm()
    recorder.expect("started")
    draw(egl, 5)
    take.finish()
    recorder.expect("finished")
    val found = packets(target)
    assertEquals(0, found.audio)
    assertEquals(recorder.keptPackets(), found.video)
    assertEquals(1, recorder.outcomes.get())
  }

  private companion object {
    const val TIMEOUT_SECONDS = 20L
    const val FRAME_NS = 33_333_333L
    const val EGL_RECORDABLE_ANDROID = 0x3142
    const val SAMPLE_RATE = 44_100
    const val FRAMES = 4_410
  }
}

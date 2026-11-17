package com.noma4i.medialayer.ui

import android.media.AudioFormat
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaRecorder
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.voiceEncoderSettings
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** The real MediaCodec and MediaMuxer under a VoiceTake, fed with a sine PCM fixture instead of a microphone. */
@RunWith(AndroidJUnit4::class)
class VoiceFileWriterDeviceTest {
  private val settings = voiceEncoderSettings()
  private val rate = settings.sampleRateHz.toInt()
  private val spec = PcmSpec(MediaRecorder.AudioSource.MIC, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private val dir = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
  private val events = LinkedBlockingQueue<String>()
  private var durationUs = 0L
  private val listener = object : VoiceTakeListener {
    override fun onStarted() { events += "started" }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { this@VoiceFileWriterDeviceTest.durationUs = durationUs; events += "finished" }
    override fun onFailed(failure: NativeFailure) { events += "failed" }
    override fun onCancelled() { events += "cancelled" }
    override fun onLost() { events += "lost" }
  }

  private fun chunk(index: Int): PcmChunk {
    val frames = 1024
    val data = ByteBuffer.allocateDirect(frames * 2).order(ByteOrder.LITTLE_ENDIAN)
    repeat(frames) { data.putShort((Math.sin(2 * Math.PI * 440 * (index * frames + it) / rate) * 12_000).toInt().toShort()) }
    data.flip()
    return PcmChunk(1, spec, data, frames, index * frames.toLong(), index * frames * 1_000_000L / rate)
  }

  private fun take(name: String) = File(dir, name).also { it.delete() }.let { it to VoiceTake(1, it, settings, Long.MAX_VALUE, MediaCodecVoiceFileWriter(), listener) }

  @Test
  fun finishGivesARealAacM4aAtTheCoreSettings() {
    val (file, take) = take("voice-device.m4a")
    assertFalse(file.exists())
    repeat(50) { take.pcm(chunk(it)) }
    assertEquals("started", events.poll(10, TimeUnit.SECONDS))
    take.finish()
    assertEquals("finished", events.poll(10, TimeUnit.SECONDS))
    val extractor = MediaExtractor().apply { setDataSource(file.path) }
    try {
      val format = extractor.getTrackFormat(0)
      assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, format.getString(MediaFormat.KEY_MIME))
      assertEquals(rate, format.getInteger(MediaFormat.KEY_SAMPLE_RATE))
      assertEquals(settings.channels.toInt(), format.getInteger(MediaFormat.KEY_CHANNEL_COUNT))
      assertEquals(1, extractor.trackCount)
    } finally {
      extractor.release()
    }
    assertTrue(durationUs in 1_000_000L..1_300_000L)
  }

  @Test
  fun cancelLeavesNoFile() {
    val (file, take) = take("voice-device-cancel.m4a")
    repeat(5) { take.pcm(chunk(it)) }
    take.cancel()
    assertEquals(listOf("started", "cancelled"), listOf(events.take(), events.take()))
    assertFalse(file.exists())
  }

  @Test
  fun cancelDuringFinishDoesNotLeaveTheFile() {
    val (file, take) = take("voice-device-cancel-finish.m4a")
    repeat(50) { take.pcm(chunk(it)) }
    take.finish()
    take.cancel()
    events.take()
    assertEquals("cancelled", events.poll(10, TimeUnit.SECONDS))
    assertFalse(file.exists())
  }
}

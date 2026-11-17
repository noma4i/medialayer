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
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VoiceFileWriterDeviceTest {
  private val settings = voiceEncoderSettings()
  private val rate = settings.sampleRateHz.toInt()
  private val spec = PcmSpec(MediaRecorder.AudioSource.MIC, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private val dir = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
  private val events = LinkedBlockingQueue<String>()
  private val durations = LinkedBlockingQueue<Long>()
  private val listener = object : VoiceTakeListener {
    override fun onStarted() { events += "started" }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { durations += durationUs; events += "finished" }
    override fun onFailed(failure: NativeFailure) { events += "failed:${failure.message}" }
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

  private class Rig(val file: File, val writer: MediaCodecVoiceFileWriter, val take: VoiceTake)

  // Each free codec input is the event that feeds the next chunk; after `chunks` the take finishes or is cancelled.
  private fun rig(name: String, chunks: Int, after: (VoiceTake) -> Unit): Rig {
    val file = File(dir, name).also { it.delete() }
    val sent = AtomicInteger()
    lateinit var take: VoiceTake
    val writer = MediaCodecVoiceFileWriter {
      val index = sent.getAndIncrement()
      if (index < chunks) take.pcm(chunk(index)) else if (index == chunks) after(take)
    }
    take = VoiceTake(1, file, settings, Long.MAX_VALUE, writer, listener)
    return Rig(file, writer, take)
  }

  @Test
  fun finishGivesARealAacM4aAtTheCoreSettings() {
    val ready = LinkedBlockingQueue<Result<Unit>>()
    val rig = rig("voice-device.m4a", 50) { it.finish() }
    rig.writer.prepare(settings, { ready += it; assertFalse(rig.file.exists()) }, rig.take::fail)
    assertEquals("started", events.poll(10, TimeUnit.SECONDS))
    assertTrue(ready.poll(10, TimeUnit.SECONDS)!!.isSuccess)
    assertEquals("finished", events.poll(10, TimeUnit.SECONDS))
    val extractor = MediaExtractor().apply { setDataSource(rig.file.path) }
    try {
      val format = extractor.getTrackFormat(0)
      assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, format.getString(MediaFormat.KEY_MIME))
      assertEquals(rate, format.getInteger(MediaFormat.KEY_SAMPLE_RATE))
      assertEquals(settings.channels.toInt(), format.getInteger(MediaFormat.KEY_CHANNEL_COUNT))
      assertEquals(1, extractor.trackCount)
    } finally {
      extractor.release()
    }
    assertTrue(durations.poll(10, TimeUnit.SECONDS)!! in 1_000_000L..1_300_000L)
  }

  @Test
  fun cancelBeforeTheFirstPcmReleasesThePreparedCodecAndLeavesNoFile() {
    val rig = rig("voice-device-early.m4a", 0) {}
    val ready = LinkedBlockingQueue<Result<Unit>>()
    rig.writer.prepare(settings, { ready += it }, rig.take::fail)
    assertTrue(ready.poll(10, TimeUnit.SECONDS)!!.isSuccess)
    rig.take.cancel()
    assertEquals("cancelled", events.poll(10, TimeUnit.SECONDS))
    assertFalse(rig.file.exists())
    assertTrue(runCatching { rig.writer.prepare(settings, {}, {}) }.isFailure)
  }

  @Test
  fun cancelWhileRecordingLeavesNoFile() {
    val rig = rig("voice-device-cancel.m4a", 5) { it.cancel() }
    rig.writer.prepare(settings, {}, rig.take::fail)
    assertEquals("started", events.poll(10, TimeUnit.SECONDS))
    assertEquals("cancelled", events.poll(10, TimeUnit.SECONDS))
    assertFalse(rig.file.exists())
  }

  @Test
  fun cancelDuringFinishLeavesNoFile() {
    val rig = rig("voice-device-cancel-finish.m4a", 50) { it.finish(); it.cancel() }
    rig.writer.prepare(settings, {}, rig.take::fail)
    assertEquals("started", events.poll(10, TimeUnit.SECONDS))
    assertEquals("cancelled", events.poll(10, TimeUnit.SECONDS))
    assertFalse(rig.file.exists())
  }
}

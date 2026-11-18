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
  private val listener = object : VoiceTakeListener {
    override fun onStarted() { events += "started" }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { events += "finished" }
    override fun onFailed(failure: NativeFailure) { events += "failed:${failure.message}" }
    override fun onCancelled() { events += "cancelled" }
    override fun onLost() { events += "lost" }
  }

  private fun chunk(index: Int): PcmChunk {
    val frames = CHUNK_FRAMES
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
  fun inv_voice_29_finishGivesARealAacM4aWithTheEncoderDelayAndNoLostAudio() {
    val ready = LinkedBlockingQueue<Result<Unit>>()
    val rig = rig("voice-device.m4a", CHUNKS) { it.finish() }
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
      extractor.selectTrack(0)
      var lastUs = 0L
      var packets = 0
      while (extractor.sampleTime >= 0) { lastUs = extractor.sampleTime; packets++; extractor.advance() }
      val frameUs = AAC_FRAME_SAMPLES * 1_000_000L / rate
      val pcmFrames = CHUNKS * CHUNK_FRAMES
      val expectedUs = pcmFrames * 1_000_000L / rate
      // MediaMuxer writes no edit list: the encoder delay (two AAC frames, flushed at the end) stays in the file as packets. Nothing is cut or added; the file is longer by that delay.
      val delayUs = AAC_ENCODER_DELAY_FRAMES * frameUs
      val durationUs = format.getLong(MediaFormat.KEY_DURATION)
      assertEquals("packets of the PCM and of the encoder delay", (pcmFrames + AAC_FRAME_SAMPLES - 1) / AAC_FRAME_SAMPLES + AAC_ENCODER_DELAY_FRAMES, packets)
      assertTrue("file duration $durationUs vs $expectedUs", durationUs in (expectedUs - frameUs)..(expectedUs + delayUs + frameUs))
      assertTrue("last packet $lastUs vs $expectedUs", lastUs + frameUs in (expectedUs - frameUs)..(expectedUs + delayUs + frameUs))
    } finally {
      extractor.release()
    }
  }

  // c2.android.aac.encoder announces its output format right after start, before the take opens the file on the first PCM.
  @Test
  fun aFormatAnnouncedBeforeTheFileOpensStillGivesAnM4a() {
    val file = File(dir, "voice-device-format-first.m4a").also { it.delete() }
    val writer = MediaCodecVoiceFileWriter()
    val ready = LinkedBlockingQueue<Result<Unit>>()
    val failures = LinkedBlockingQueue<Throwable>()
    writer.prepare(settings, { ready += it }, { failures += it })
    assertTrue(ready.poll(10, TimeUnit.SECONDS)!!.isSuccess)
    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
    while (!writer.formatKnown && System.nanoTime() < deadline) Thread.sleep(10)
    assertTrue("the encoder announced no format before open", writer.formatKnown)
    writer.open(file)
    writer.append(chunk(0))
    val done = LinkedBlockingQueue<Result<Long>>()
    writer.finish { done += it }
    val result = done.poll(10, TimeUnit.SECONDS)!!
    assertTrue("finish failed: ${result.exceptionOrNull()}", result.isSuccess)
    assertTrue(failures.isEmpty())
    val extractor = MediaExtractor().apply { setDataSource(file.path) }
    try {
      assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, extractor.getTrackFormat(0).getString(MediaFormat.KEY_MIME))
    } finally {
      extractor.release()
    }
  }

  // A codec that announces the same output format again after the muxer started (O-08) is no failure; a different one is.
  @Test
  fun inv_voice_32_aReannouncedSameFormatKeepsTheTakeAndADifferentOneFailsIt() {
    val file = File(dir, "voice-device-reannounce.m4a").also { it.delete() }
    val writer = MediaCodecVoiceFileWriter()
    val failures = LinkedBlockingQueue<Throwable>()
    val ready = LinkedBlockingQueue<Result<Unit>>()
    writer.prepare(settings, { ready += it }, { failures += it })
    assertTrue(ready.poll(10, TimeUnit.SECONDS)!!.isSuccess)
    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
    while (!writer.formatKnown && System.nanoTime() < deadline) Thread.sleep(10)
    writer.open(file)
    val announced = writer.announcedFormat!!
    writer.outputFormat(announced)
    writer.append(chunk(0))
    val done = LinkedBlockingQueue<Result<Long>>()
    writer.finish { done += it }
    assertTrue("same format failed: ${done.poll(10, TimeUnit.SECONDS)?.exceptionOrNull()}", done.isEmpty() || failures.isEmpty())
    assertTrue(failures.isEmpty())
    assertTrue(file.exists())
    val other = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rate / 2, settings.channels.toInt())
    val second = MediaCodecVoiceFileWriter()
    val secondFailures = LinkedBlockingQueue<Throwable>()
    val secondReady = LinkedBlockingQueue<Result<Unit>>()
    second.prepare(settings, { secondReady += it }, { secondFailures += it })
    assertTrue(secondReady.poll(10, TimeUnit.SECONDS)!!.isSuccess)
    val limit = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
    while (!second.formatKnown && System.nanoTime() < limit) Thread.sleep(10)
    second.open(File(dir, "voice-device-reannounce-other.m4a").also { it.delete() })
    second.outputFormat(other)
    assertTrue("a different format must fail the take", secondFailures.poll(10, TimeUnit.SECONDS) != null)
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

  private companion object {
    const val CHUNKS = 50
    const val CHUNK_FRAMES = 1024
    const val AAC_FRAME_SAMPLES = 1024
    const val AAC_ENCODER_DELAY_FRAMES = 2
  }
}

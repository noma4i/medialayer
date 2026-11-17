package com.noma4i.medialayer.ui

import android.media.MediaFormat
import androidx.camera.core.impl.EncoderProfilesProxy.AudioProfileProxy
import androidx.camera.core.impl.EncoderProfilesProxy.VideoProfileProxy
import androidx.camera.video.internal.VideoValidatedEncoderProfilesProxy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CameraEncodersTest {
  private val sdr = VideoProfileProxy.create(2, "video/avc", 8_000_000, 30, 1280, 720, 1, 8, 1, 0)
  private val hdr = VideoProfileProxy.create(5, "video/hevc", 8_000_000, 30, 1280, 720, 2, 10, 1, 2)
  private val aac = AudioProfileProxy.create(3, MediaFormat.MIMETYPE_AUDIO_AAC, 128_000, 48_000, 2, 2)

  private fun set(video: List<VideoProfileProxy>, sound: List<AudioProfileProxy>) = VideoValidatedEncoderProfilesProxy.create(30, 2, sound, video)

  @Test
  fun a_camera_the_legacy_profile_has_none_for_records_with_the_profile_of_the_camerax_provider() {
    val made = encodersFrom(set(listOf(hdr, sdr), listOf(aac)), true, VideoTimebase.BOOTTIME)!!
    assertEquals("video/avc", made.videoMime)
    assertEquals(1280, made.width)
    assertEquals(8_000_000, made.videoBitrate)
    assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, made.audioMime)
    assertEquals(48_000, made.sampleRate)
  }

  @Test
  fun a_provider_without_an_sdr_video_or_without_a_profile_is_an_honest_absence() {
    assertNull(encodersFrom(set(listOf(hdr), listOf(aac)), true, VideoTimebase.BOOTTIME))
    assertNull(encodersFrom(null, false, VideoTimebase.BOOTTIME))
  }

  @Test
  fun each_refusal_of_the_plan_names_its_own_step_and_the_facts_it_stopped_at() {
    val audio = encodersFrom(set(listOf(sdr), listOf(aac)), true, VideoTimebase.BOOTTIME)!!
    val facts = PlanFacts("1", "HD", listOf("SD", "HD"), 2, 1, audio)
    val texts = PlanRefusal.values().map { PlanRefused(it, facts).message!! }
    assertEquals(PlanRefusal.values().size, texts.toSet().size)
    texts.forEachIndexed { index, text -> assertTrue(text, text.startsWith(PlanRefusal.values()[index].step)) }
    val text = texts.first()
    listOf("camera 1", "quality HD", "supported [SD, HD]", "video 2 audio 1", "audio audio/mp4a-latm 48000Hz 2ch profile 2 128000bps").forEach { assertTrue(text, text.contains(it)) }
    val early = PlanRefused(PlanRefusal.NO_STREAM_MAP, PlanFacts("0", null, emptyList(), null, null, null)).message!!
    assertTrue(early, early.contains("quality none") && early.contains("video none audio none") && early.contains("audio none"))
  }

  @Test
  fun a_provider_without_aac_gets_the_default_aac_of_the_core_rule() {
    val made = encodersFrom(set(listOf(sdr), emptyList()), true, VideoTimebase.BOOTTIME)!!
    assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, made.audioMime)
    assertEquals(44_100, made.sampleRate)
    assertEquals(1, made.channels)
  }

  @Test
  fun the_aac_profile_is_the_one_of_the_camera_when_an_encoder_declares_it_else_lc_else_none() {
    val lc = android.media.MediaCodecInfo.CodecProfileLevel.AACObjectLC
    val main = android.media.MediaCodecInfo.CodecProfileLevel.AACObjectMain
    assertEquals(lc, resolveAacProfile(main, setOf(lc)))
    assertEquals(lc, resolveAacProfile(lc, setOf(lc)))
    assertEquals(main, resolveAacProfile(main, setOf(lc, main)))
    assertNull(resolveAacProfile(main, emptySet()))
    assertNull(resolveAacProfile(lc, setOf(main)))
  }
}

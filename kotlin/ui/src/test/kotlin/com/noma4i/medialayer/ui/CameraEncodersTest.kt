package com.noma4i.medialayer.ui

import android.media.MediaFormat
import androidx.camera.core.impl.EncoderProfilesProxy.AudioProfileProxy
import androidx.camera.core.impl.EncoderProfilesProxy.VideoProfileProxy
import androidx.camera.video.internal.VideoValidatedEncoderProfilesProxy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
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
  fun a_provider_without_aac_gets_the_default_aac_of_the_core_rule() {
    val made = encodersFrom(set(listOf(sdr), emptyList()), true, VideoTimebase.BOOTTIME)!!
    assertEquals(MediaFormat.MIMETYPE_AUDIO_AAC, made.audioMime)
    assertEquals(44_100, made.sampleRate)
    assertEquals(1, made.channels)
  }
}

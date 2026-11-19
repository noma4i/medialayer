package com.noma4i.medialayer.ui

import java.nio.ByteBuffer

/** The frame of one AAC access unit in PCM samples per channel for the AAC-LC the writers of the camera produce: the AudioSpecificConfig is read by media3 `AacUtil` (the object type is its `mp4a.40.2` codec string, the frame is `AAC_LC_AUDIO_SAMPLE_COUNT`) and its rate must be the rate the decoder outputs, so a signalled SBR or PS that doubles the output is not taken for LC. Any other profile or a config that does not parse is 0: the core then refuses the clip as broken. */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
internal fun aacFrameSamples(config: ByteBuffer?, outputRate: Int): Int {
  if (config == null) return 0
  val bytes = ByteArray(config.remaining()).also { config.duplicate().get(it) }
  val parsed = try { androidx.media3.extractor.AacUtil.parseAudioSpecificConfig(bytes) } catch (_: androidx.media3.common.ParserException) { return 0 }
  return if (parsed.codecs == "mp4a.40.${androidx.media3.extractor.AacUtil.AUDIO_OBJECT_TYPE_AAC_LC}" && parsed.sampleRateHz == outputRate) androidx.media3.extractor.AacUtil.AAC_LC_AUDIO_SAMPLE_COUNT else 0
}

package com.noma4i.medialayer.ui

import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File

internal interface VoiceFileWriter {
  fun open(file: File, settings: VoiceEncoderSettings)

  fun append(chunk: PcmChunk)

  fun finish(done: (Result<Long>) -> Unit)

  fun cancel()
}

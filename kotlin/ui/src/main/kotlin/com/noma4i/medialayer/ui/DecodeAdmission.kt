package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer

internal typealias DecodeAdmit = suspend (Long) -> AutoCloseable

internal fun MediaLayer.decodeAdmit(): DecodeAdmit = { bytes ->
  val hold = reserveDecode(bytes.toULong())
  AutoCloseable { hold.release() }
}

internal fun MediaLayer.viewDecodeAdmit(): DecodeAdmit = { bytes -> admitReporting(decodeAdmit(), bytes) }

internal suspend fun admitReporting(admit: DecodeAdmit, bytes: Long): AutoCloseable =
  try {
    admit(bytes)
  } catch (refused: MediaException) {
    Media.failure = refused
    throw refused
  }

package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import java.lang.ref.WeakReference
import kotlinx.coroutines.CancellationException

internal typealias DecodeAdmit = suspend (Long) -> AutoCloseable

internal fun MediaLayer.decodeAdmit(): DecodeAdmit {
  val layer = WeakReference(this)
  return { bytes ->
    val hold = (layer.get() ?: throw CancellationException("MediaLayer was released")).reserveDecode(bytes.toULong())
    AutoCloseable { hold.release() }
  }
}

internal fun MediaLayer.viewDecodeAdmit(): DecodeAdmit {
  val admit = decodeAdmit()
  return { bytes -> admitReporting(admit, bytes) }
}

internal suspend fun admitReporting(admit: DecodeAdmit, bytes: Long): AutoCloseable =
  try {
    admit(bytes)
  } catch (refused: MediaException) {
    Media.failure = refused
    throw refused
  }

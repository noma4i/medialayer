package com.noma4i.medialayer.ui

import com.noma4i.medialayer.isVolumeRaise
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

internal fun VolumePort.raises(): Flow<Unit> = flow {
  val reading = reading() ?: return@flow
  var previous = reading.initial
  reading.changes.collect { current ->
    if (isVolumeRaise(previous, current)) emit(Unit)
    previous = current
  }
}

package com.noma4i.medialayer

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate

fun MediaLayer.statsFlow(): Flow<MediaStats> = callbackFlow {
  val subscription = try {
    subscribeStats(object : StatsListener {
      override fun changed(stats: MediaStats) { trySend(stats) }
      override fun failed(error: MediaException) { close() }
    })
  } catch (error: MediaException) {
    close()
    null
  }
  awaitClose { subscription?.close() }
}.conflate()

fun MediaLayer.stateFlow(key: String): Flow<MediaState> = callbackFlow {
  val subscription = try {
    subscribeState(key, object : StateListener {
      override fun changed(state: MediaState) { trySend(state) }
      override fun failed(error: MediaException) { trySend(MediaState.Failed(error.toString())); close() }
    })
  } catch (error: MediaException) {
    trySend(MediaState.Failed(error.toString()))
    close()
    null
  }
  awaitClose { subscription?.close() }
}.conflate()

val MediaState.isReady: Boolean get() = this is MediaState.Ready || this is MediaState.Rendered || this is MediaState.Ended

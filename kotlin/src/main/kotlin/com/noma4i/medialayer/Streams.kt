package com.noma4i.medialayer

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate

fun MediaLayer.statsFlow(): Flow<MediaStats> = callbackFlow {
  val subscription = subscribeStats(object : StatsListener {
    override fun changed(stats: MediaStats) { trySend(stats) }
  })
  awaitClose { subscription.close() }
}.conflate()

fun MediaLayer.stateFlow(key: String): Flow<MediaState> = callbackFlow {
  val subscription = subscribeState(key, object : StateListener {
    override fun changed(state: MediaState) { trySend(state) }
  })
  awaitClose { subscription.close() }
}.conflate()

val MediaState.isReady: Boolean get() = this is MediaState.Ready || this is MediaState.Rendered || this is MediaState.Ended

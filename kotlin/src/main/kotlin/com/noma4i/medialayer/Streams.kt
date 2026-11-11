package com.noma4i.medialayer

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate

suspend fun MediaLayer.imageSize(source: MediaSource): ImageSize? = imageSize(source.url, source.headers)

/** Every load failure and every cache store failure, one value each. Cancelling the collection cancels the watch. */
fun MediaLayer.failureFlow(): Flow<MediaFailure> = callbackFlow {
  val subscription = subscribeFailures(object : FailureListener {
    override fun failed(failure: MediaFailure) { trySend(failure) }
  })
  awaitClose { subscription.close() }
}.buffer(Channel.UNLIMITED)

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

sealed interface ToneUpdate {
  data class Value(val tone: MediaTone?) : ToneUpdate
  data class Failed(val error: MediaException) : ToneUpdate
}

/** The tone of [source] at [band]: the value after the watch is set, then after every change. Cancelling the collector cancels the watch. */
fun MediaLayer.toneFlow(source: MediaItem, band: Float): Flow<ToneUpdate> = callbackFlow {
  val subscription = try {
    subscribeTone(source, band, object : ToneListener {
      override fun changed(tone: MediaTone?) { trySend(ToneUpdate.Value(tone)) }
      override fun failed(error: MediaException) { trySend(ToneUpdate.Failed(error)); close() }
    })
  } catch (error: MediaException) {
    trySend(ToneUpdate.Failed(error))
    close()
    null
  }
  awaitClose { subscription?.close() }
}.conflate()

val MediaState.isFailed: Boolean get() = this is MediaState.Failed

val MediaState.isReady: Boolean get() = this is MediaState.Ready || this is MediaState.Rendered || this is MediaState.Ended

val MediaState.isRendered: Boolean get() = this is MediaState.Rendered || this is MediaState.Ended

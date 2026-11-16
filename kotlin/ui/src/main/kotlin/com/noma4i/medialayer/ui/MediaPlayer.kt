package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaPlayer
import com.noma4i.medialayer.PlaybackListener
import com.noma4i.medialayer.PlaybackState
import com.noma4i.medialayer.Subscription
import java.util.WeakHashMap
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.transformWhile

private class PlayerState(val state: StateFlow<PlaybackState>, val closed: MutableStateFlow<Boolean>) {
  var subscription: Subscription? = null
}
private val playerStates = WeakHashMap<MediaPlayer, PlayerState>()

private val MediaPlayer.playerState: PlayerState
  get() = synchronized(playerStates) {
    playerStates.getOrPut(this) {
      val current = MutableStateFlow(PlaybackState(false, false, null, null, 0uL))
      val closed = MutableStateFlow(false)
      val holder = PlayerState(current, closed)
      val listener = object : PlaybackListener {
        override fun changed(state: PlaybackState) { current.value = state }
        override fun failed(error: MediaException) {
          if (error !is MediaException.Closed) Media.recordFailure(error)
          closed.value = true
          synchronized(holder) { holder.subscription?.close(); holder.subscription = null }
        }
      }
      inputExecutor.execute {
        try {
          val subscription = subscribePlayback(listener)
          synchronized(holder) { if (closed.value) subscription.close() else holder.subscription = subscription }
        } catch (error: MediaException) {
          listener.failed(error)
        }
      }
      holder
    }
  }

val MediaPlayer.state: StateFlow<PlaybackState> get() = playerState.state
val MediaPlayer.states: Flow<PlaybackState> get() = playerState.let { holder ->
  combine(holder.state, holder.closed) { state, closed -> state to closed }
    .transformWhile { (state, closed) -> emit(state); !closed }
}

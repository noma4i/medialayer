package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.SlotCall
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

internal fun ExoPlayerSlot.post(call: SlotCall) {
  CoroutineScope(Dispatchers.Unconfined).launch { apply(call) }
}

internal fun ExoPlayerSlot.disposing() {
  CoroutineScope(Dispatchers.Unconfined).launch { dispose() }
}

internal fun ExoPlayerSlot.load(key: String, route: PlaybackRoute, generation: ULong = 1uL) = post(SlotCall.Load(key, route, generation))
internal fun ExoPlayerSlot.unload() = post(SlotCall.Unload)
internal fun ExoPlayerSlot.preroll() = post(SlotCall.Preroll)
internal fun ExoPlayerSlot.play() = post(SlotCall.Play)
internal fun ExoPlayerSlot.pause() = post(SlotCall.Pause)
internal fun ExoPlayerSlot.mute() = post(SlotCall.Mute)
internal fun ExoPlayerSlot.unmute() = post(SlotCall.Unmute)
internal fun ExoPlayerSlot.detach() = post(SlotCall.Detach)
internal fun ExoPlayerSlot.attach(viewToken: ULong) = post(SlotCall.Attach(viewToken))
internal fun ExoPlayerSlot.seek(positionMs: ULong) = post(SlotCall.Seek(positionMs))
internal fun ExoPlayerSlot.setRate(rate: Float) = post(SlotCall.Rate(rate))
internal fun ExoPlayerSlot.setReadahead(ms: ULong) = post(SlotCall.Readahead(ms))

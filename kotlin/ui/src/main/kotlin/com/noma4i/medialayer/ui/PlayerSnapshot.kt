package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaLayer

data class PlayerSnapshot(val key: String, val isPlaying: Boolean, val isMuted: Boolean, val positionMs: Long, val rate: Float)

val MediaLayer.slotPlayers: SlotPlayers? get() = binding?.factory?.players

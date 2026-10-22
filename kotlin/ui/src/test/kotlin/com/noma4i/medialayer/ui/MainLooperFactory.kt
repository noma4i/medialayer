package com.noma4i.medialayer.ui

import android.os.Looper

internal fun ExoPlayerSlotFactory.onMainLooper() = also { it.looperFor = { Looper.getMainLooper() } }

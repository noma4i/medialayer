package com.noma4i.medialayer.ui

import android.content.Context
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate

class MediaVolumeReading internal constructor(val initial: Float, val changes: Flow<Float>)

internal interface VolumePort {
  fun level(): Float?

  fun observe(onChange: () -> Unit): () -> Unit
}

internal class SystemVolume(context: Context) : VolumePort {
  private val resolver = context.applicationContext.contentResolver
  private val manager = context.applicationContext.getSystemService(AudioManager::class.java)

  override fun level(): Float? {
    val max = manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
    if (max <= 0) return null

    return manager.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat() / max
  }

  override fun observe(onChange: () -> Unit): () -> Unit {
    val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
      override fun onChange(selfChange: Boolean) = onChange()
    }
    resolver.registerContentObserver(Settings.System.CONTENT_URI, true, observer)
    return { resolver.unregisterContentObserver(observer) }
  }
}

internal fun VolumePort.reading(): MediaVolumeReading? {
  val initial = level() ?: return null
  val changes = callbackFlow {
    var last = initial
    val publish = { level()?.takeIf { it != last }?.let { last = it; trySend(it) }; Unit }
    val stop = observe(publish)
    publish()
    awaitClose { stop() }
  }.conflate()
  return MediaVolumeReading(initial, changes)
}

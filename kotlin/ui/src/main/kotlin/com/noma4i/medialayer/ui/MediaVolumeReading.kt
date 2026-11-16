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
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.channels.Channel

class MediaVolumeReading internal constructor(val initial: Float, val changes: Flow<Float>)

internal interface VolumePort {
  fun level(): Float?

  fun observe(onChange: () -> Unit): () -> Unit
}

internal class SystemVolume(context: Context) : VolumePort {
  lateinit var handler: Handler
  private val resolver = context.applicationContext.contentResolver
  private val manager = context.applicationContext.getSystemService(AudioManager::class.java)

  override fun level(): Float? {
    val max = manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
    if (max <= 0) return null

    return manager.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat() / max
  }

  override fun observe(onChange: () -> Unit): () -> Unit {
    val observer = object : ContentObserver(handler) {
      override fun onChange(selfChange: Boolean) = onChange()
    }
    resolver.registerContentObserver(Settings.System.CONTENT_URI, true, observer)
    return { resolver.unregisterContentObserver(observer) }
  }
}

internal fun VolumePort.reading(initial: Float, handler: Handler): MediaVolumeReading {
  val changes = callbackFlow {
    var last = initial
    val read = { level()?.takeIf { it != last }?.let { last = it; trySend(it) }; Unit }
    val publish = { if (Looper.myLooper() == handler.looper) read() else { handler.post { read() }; Unit } }
    val stop = observe(publish)
    publish()
    awaitClose { stop() }
  }.buffer(Channel.UNLIMITED)
  return MediaVolumeReading(initial, changes)
}

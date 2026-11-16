package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager

internal class SystemAudioFocus(context: Context) : AudioFocusPort {
  private val manager = context.applicationContext.getSystemService(AudioManager::class.java)
  private var current: AudioFocusRequest? = null
  private val lost = java.util.concurrent.atomic.AtomicInteger()
  @Volatile private var onChange: () -> Unit = {}
  override val losses: Int get() = lost.get()

  override fun onChange(action: () -> Unit) { onChange = action }

  override fun request(kind: FocusKind): Boolean {
    val gain = when (kind) {
      FocusKind.Duck -> AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK
      FocusKind.Recording -> AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE
    }
    val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build()
    val request = AudioFocusRequest.Builder(gain).setAudioAttributes(attributes).setOnAudioFocusChangeListener { change ->
      if (change < 0) {
        lost.incrementAndGet()
        MediaTrace.emit("audio") { "focus lost $change" }
      }
      onChange()
    }.build()
    if (manager.requestAudioFocus(request) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) return false

    val previous = current
    current = request
    previous?.let(manager::abandonAudioFocusRequest)
    return true
  }

  override fun abandon() {
    current?.let(manager::abandonAudioFocusRequest)
    current = null
  }
}

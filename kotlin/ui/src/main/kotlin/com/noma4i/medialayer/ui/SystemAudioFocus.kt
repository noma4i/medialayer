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
      FocusKind.Call -> AudioManager.AUDIOFOCUS_GAIN
    }
    val attributes = AudioAttributes.Builder()
      .setUsage(if (kind == FocusKind.Call) AudioAttributes.USAGE_VOICE_COMMUNICATION else AudioAttributes.USAGE_MEDIA)
      .setContentType(if (kind == FocusKind.Call) AudioAttributes.CONTENT_TYPE_SPEECH else AudioAttributes.CONTENT_TYPE_MOVIE).build()
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

  override fun setCommunication(on: Boolean, speaker: Boolean) {
    if (on) {
      manager.mode = AudioManager.MODE_IN_COMMUNICATION
      val device = manager.availableCommunicationDevices.firstOrNull { it.type == android.media.AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
      if (speaker && device != null) manager.setCommunicationDevice(device) else manager.clearCommunicationDevice()
    } else {
      manager.clearCommunicationDevice()
      manager.mode = AudioManager.MODE_NORMAL
    }
  }
}

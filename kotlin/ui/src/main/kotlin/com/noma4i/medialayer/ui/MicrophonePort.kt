package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioManager
import android.media.AudioRecordingConfiguration
import android.os.Handler
import android.os.Looper
import android.telephony.TelephonyCallback
import android.telephony.TelephonyManager
import com.noma4i.medialayer.CameraRefusal
import com.noma4i.medialayer.VoiceRefusal

/** Recording configurations are anonymous, so active configurations count as another app only without our recording or call token. */
enum class MicrophoneAvailability { AVAILABLE, OWN_CALL, EXTERNAL_CALL, OTHER_APP, PERMISSION_DENIED }

internal fun MicrophoneAvailability.voiceRefusal(): VoiceRefusal = when (this) {
  MicrophoneAvailability.OWN_CALL, MicrophoneAvailability.EXTERNAL_CALL -> VoiceRefusal.IN_CALL
  MicrophoneAvailability.PERMISSION_DENIED -> VoiceRefusal.PERMISSION_DENIED
  else -> VoiceRefusal.MICROPHONE_BUSY
}

internal fun MicrophoneAvailability.cameraRefusal(): CameraRefusal = when (this) {
  MicrophoneAvailability.OWN_CALL, MicrophoneAvailability.EXTERNAL_CALL -> CameraRefusal.IN_CALL
  MicrophoneAvailability.PERMISSION_DENIED -> CameraRefusal.PERMISSION_DENIED
  else -> CameraRefusal.MICROPHONE_BUSY
}

internal data class MicrophoneState(
  val externalCall: Boolean = false,
  val inputAvailable: Boolean = true,
  val otherRecording: Boolean = false,
  val permissionGranted: Boolean = true,
  val silenced: Boolean = false
)

internal interface MicrophonePort : AutoCloseable {
  fun subscribe(change: (MicrophoneState, String) -> Unit)
  fun foreground()
}

internal object AvailableMicrophone : MicrophonePort {
  override fun subscribe(change: (MicrophoneState, String) -> Unit) = change(MicrophoneState(), "subscribe")
  override fun foreground() = Unit
  override fun close() = Unit
}

internal class SystemMicrophone(private val context: Context) : MicrophonePort {
  private val audio = context.getSystemService(AudioManager::class.java)
  private val telephony = context.getSystemService(TelephonyManager::class.java)
  private val handler = Handler(Looper.getMainLooper())
  private var change: ((MicrophoneState, String) -> Unit)? = null
  private var phoneRegistered = false
  private var phoneCall = false
  private val modeListener = AudioManager.OnModeChangedListener { publish("mode") }
  private val recordingListener = object : AudioManager.AudioRecordingCallback() {
    override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) = publish("recording")
  }
  private val phoneListener = object : TelephonyCallback(), TelephonyCallback.CallStateListener {
    override fun onCallStateChanged(state: Int) {
      phoneCall = state != TelephonyManager.CALL_STATE_IDLE
      publish("telephony")
    }
  }

  override fun subscribe(change: (MicrophoneState, String) -> Unit) {
    this.change = change
    audio.addOnModeChangedListener(context.mainExecutor, modeListener)
    audio.registerAudioRecordingCallback(recordingListener, handler)
    refreshPhone()
    publish("subscribe")
  }

  override fun foreground() {
    refreshPhone()
    publish("foreground")
  }

  private fun refreshPhone() {
    val allowed = context.checkSelfPermission(Manifest.permission.READ_PHONE_STATE) == PackageManager.PERMISSION_GRANTED
    if (telephony == null) return
    if (allowed && !phoneRegistered) {
      telephony.registerTelephonyCallback(context.mainExecutor, phoneListener)
      phoneRegistered = true
    } else if (!allowed && phoneRegistered) {
      telephony.unregisterTelephonyCallback(phoneListener)
      phoneRegistered = false
      phoneCall = false
    }
  }

  private fun publish(source: String) {
    val mode = audio.mode
    val configs = audio.activeRecordingConfigurations
    change?.invoke(MicrophoneState(
      externalCall = phoneCall || mode == AudioManager.MODE_IN_CALL || mode == AudioManager.MODE_IN_COMMUNICATION || mode == AudioManager.MODE_CALL_SCREENING,
      otherRecording = configs.isNotEmpty(),
      permissionGranted = context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED,
      silenced = configs.any { it.isClientSilenced }
    ), source)
  }

  override fun close() {
    change = null
    audio.removeOnModeChangedListener(modeListener)
    audio.unregisterAudioRecordingCallback(recordingListener)
    if (phoneRegistered) telephony?.unregisterTelephonyCallback(phoneListener)
    phoneRegistered = false
  }
}

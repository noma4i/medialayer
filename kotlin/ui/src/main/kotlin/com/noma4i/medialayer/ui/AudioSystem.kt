package com.noma4i.medialayer.ui

import android.Manifest
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.AudioRecordingConfiguration
import android.os.Handler
import android.telephony.TelephonyCallback
import android.telephony.TelephonyManager
import androidx.core.content.ContextCompat
import com.noma4i.medialayer.FocusAnswer
import com.noma4i.medialayer.FocusChange

internal class AudioSystem(private val context: Context) : AudioFocusPort, AutoCloseable {
  lateinit var handler: Handler
  private val audio = context.getSystemService(AudioManager::class.java)
  private val phone = context.getSystemService(TelephonyManager::class.java)
  private var sink: (FocusSignal) -> Unit = {}
  private var phoneRegistered = false
  private var ownRecording = false
  private val executor = java.util.concurrent.Executor { handler.post(it) }
  private val mode = AudioManager.OnModeChangedListener { value ->
    sink(FocusSignal.CommunicationMode(value == AudioManager.MODE_IN_COMMUNICATION || value == AudioManager.MODE_IN_CALL))
  }
  private val recording = object : AudioManager.AudioRecordingCallback() {
    override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
      if (!ownRecording) sink(FocusSignal.OtherRecording(configs.isNotEmpty()))
    }
  }
  private val phoneListener = object : TelephonyCallback(), TelephonyCallback.CallStateListener {
    override fun onCallStateChanged(state: Int) { sink(FocusSignal.PhoneCall(state != TelephonyManager.CALL_STATE_IDLE)) }
  }
  private val noisy = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { sink(FocusSignal.Noisy) }
  }
  override fun listen(sink: (FocusSignal) -> Unit) {
    this.sink = sink
    audio.addOnModeChangedListener(executor, mode)
    audio.registerAudioRecordingCallback(recording, handler)
    ContextCompat.registerReceiver(context, noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_NOT_EXPORTED)
    refreshPhone()
    mode.onModeChanged(audio.mode)
    sink(FocusSignal.OtherRecording(audio.activeRecordingConfigurations.isNotEmpty()))
  }
  fun ownRecording(on: Boolean) {
    ownRecording = on
    if (!on) sink(FocusSignal.OtherRecording(audio.activeRecordingConfigurations.isNotEmpty()))
  }
  fun refreshPhone() {
    val allowed = context.checkSelfPermission(Manifest.permission.READ_PHONE_STATE) == PackageManager.PERMISSION_GRANTED
    if (phone != null && allowed && !phoneRegistered) { phone.registerTelephonyCallback(executor, phoneListener); phoneRegistered = true }
    if (phone != null && !allowed && phoneRegistered) { phone.unregisterTelephonyCallback(phoneListener); phoneRegistered = false; sink(FocusSignal.PhoneCall(false)) }
  }
  override fun request(plan: FocusPlan): FocusAnswer = focusOperation {
    val previous = active
    previous?.let { it.owner.audio.abandonAudioFocusRequest(it.request) }
    active = null
    val gain = when (plan.gain) {
      FocusGain.GAIN -> AudioManager.AUDIOFOCUS_GAIN
      FocusGain.GAIN_TRANSIENT -> AudioManager.AUDIOFOCUS_GAIN_TRANSIENT
      FocusGain.GAIN_TRANSIENT_EXCLUSIVE -> AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE
    }
    val usage = if (plan.usage == FocusUsage.MEDIA) AudioAttributes.USAGE_MEDIA else AudioAttributes.USAGE_VOICE_COMMUNICATION
    val attributes = AudioAttributes.Builder().setUsage(usage).build()
    val identity = Any()
    val request = AudioFocusRequest.Builder(gain).setAudioAttributes(attributes).setAcceptsDelayedFocusGain(true)
      .setOnAudioFocusChangeListener({ value ->
        val current = focusOperation {
          active?.let { it.owner === this && it.identity === identity } == true
        }
        if (!current) return@setOnAudioFocusChangeListener
        val change = when (value) {
          AudioManager.AUDIOFOCUS_GAIN -> FocusChange.GAIN
          AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> FocusChange.LOSS_TRANSIENT
          AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> FocusChange.LOSS_TRANSIENT_DUCK
          else -> FocusChange.LOSS
        }
        sink(FocusSignal.Change(change))
      }, handler).build()
    val answer = when (audio.requestAudioFocus(request)) {
      AudioManager.AUDIOFOCUS_REQUEST_GRANTED -> FocusAnswer.GRANTED
      AudioManager.AUDIOFOCUS_REQUEST_DELAYED -> FocusAnswer.DELAYED
      else -> FocusAnswer.FAILED
    }
    if (answer != FocusAnswer.FAILED) active = ActiveFocus(this, request, identity)
    answer
  }
  override fun abandon() = focusOperation {
    active?.takeIf { it.owner === this }?.let { audio.abandonAudioFocusRequest(it.request); active = null }
    Unit
  }
  override fun enterCommunication(speaker: Boolean) {
    audio.mode = AudioManager.MODE_IN_COMMUNICATION
    val device = audio.availableCommunicationDevices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
    if (speaker && device != null) audio.setCommunicationDevice(device) else audio.clearCommunicationDevice()
  }
  override fun leaveCommunication() {
    var failure: Exception? = null
    try { audio.mode = AudioManager.MODE_NORMAL } catch (error: Exception) { failure = error }
    try { audio.clearCommunicationDevice() } catch (error: Exception) {
      if (failure == null) failure = error else failure.addSuppressed(error)
    }
    failure?.let { throw it }
  }
  override fun close() {
    abandon()
    audio.removeOnModeChangedListener(mode)
    audio.unregisterAudioRecordingCallback(recording)
    if (phoneRegistered) phone?.unregisterTelephonyCallback(phoneListener)
    context.unregisterReceiver(noisy)
  }
  private data class ActiveFocus(val owner: AudioSystem, val request: AudioFocusRequest, val identity: Any)
  private companion object {
    val focusTail = java.util.concurrent.atomic.AtomicReference(java.util.concurrent.CompletableFuture.completedFuture(Unit))
    var active: ActiveFocus? = null
    fun <T> focusOperation(work: () -> T): T {
      val done = java.util.concurrent.CompletableFuture<Unit>()
      val previous = focusTail.getAndSet(done)
      try { previous.join(); return work() } finally { done.complete(Unit) }
    }
  }
}

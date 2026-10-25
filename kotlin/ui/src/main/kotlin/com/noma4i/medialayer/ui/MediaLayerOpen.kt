package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.BroadcastReceiver
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.Configuration
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.os.Trace
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MemoryReleaser
import com.noma4i.medialayer.NativeCallbacks
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.PlayerEventsInterface
import com.noma4i.medialayer.Pressure
import com.noma4i.medialayer.SettingsStore
import com.noma4i.medialayer.SpecDefaults
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.specDefaults
import java.io.File
import java.util.Collections
import java.util.WeakHashMap
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.ensureActive
import coil3.ImageLoader
import okhttp3.Call
import okhttp3.OkHttpClient

internal class HeldResources(val signals: SystemSignals?, val factory: ExoPlayerSlotFactory?, val imageLoader: ImageLoader?, val layer: MediaLayer?)

internal class Binding(val factory: ExoPlayerSlotFactory, val signals: SystemSignals, val imageLoader: ImageLoader)

private val bindings: MutableMap<MediaLayer, Binding> = Collections.synchronizedMap(WeakHashMap())

private val openScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

internal val MediaLayer.binding: Binding? get() = bindings[this]

fun MediaLayer.Companion.open(
  context: Context,
  fetcher: Fetcher? = null,
  settings: SettingsStore,
  callFactory: Call.Factory? = null,
  silent: Boolean = false,
  audioFocus: MediaAudioFocus = MediaAudioFocus.Mix,
  imageMemoryBytes: Int? = null,
  videoOutputCounters: VideoOutputCounters? = null
): Deferred<MediaLayer> =
  openMediaLayer(context, fetcher, settings, callFactory, silent = silent, lifecycle = ProcessLifecycleOwner.get().lifecycle, audioFocus = audioFocus, imageMemoryBytes = imageMemoryBytes, videoOutputCounters = videoOutputCounters) { MediaLayer.open(it) }

fun MediaLayer.Companion.open(
  context: Context,
  spec: String,
  fetcher: Fetcher? = null,
  settings: SettingsStore,
  callFactory: Call.Factory? = null,
  silent: Boolean = false,
  audioFocus: MediaAudioFocus = MediaAudioFocus.Mix,
  imageMemoryBytes: Int? = null,
  videoOutputCounters: VideoOutputCounters? = null
): Deferred<MediaLayer> =
  openMediaLayer(context, fetcher, settings, callFactory, defaults = { specDefaults(spec) }, silent = silent, lifecycle = ProcessLifecycleOwner.get().lifecycle, audioFocus = audioFocus, imageMemoryBytes = imageMemoryBytes, videoOutputCounters = videoOutputCounters) { MediaLayer.open(it) }

private fun platformDefaults(): SpecDefaults = SpecDefaults(defaultLimits(), defaultPoolSettings(), emptyList())

internal fun openMediaLayer(
  context: Context,
  fetcher: Fetcher? = null,
  settings: SettingsStore,
  callFactory: Call.Factory? = null,
  defaults: () -> SpecDefaults = ::platformDefaults,
  silent: Boolean = false,
  lifecycle: Lifecycle,
  audioFocus: MediaAudioFocus = MediaAudioFocus.Mix,
  imageMemoryBytes: Int? = null,
  videoOutputCounters: VideoOutputCounters? = null,
  install: () -> Unit = NativeCallbacks::install,
  open: (OpenConfig) -> MediaLayer
): Deferred<MediaLayer> {
  val app = context.applicationContext
  val audio = MediaAudioSession(app, audioFocus)
  val lock = Any()
  var factory: ExoPlayerSlotFactory? = null
  var reference: WeakReference<MediaLayer>? = null
  var imageLoader: ImageLoader? = null
  var signals: SystemSignals? = null
  fun detach(): HeldResources = synchronized(lock) {
    HeldResources(signals, factory, imageLoader, reference?.get()).also {
      signals = null
      factory = null
      imageLoader = null
    }
  }
  fun finish(held: HeldResources) {
    held.factory?.release() ?: audio.close()
    held.imageLoader?.shutdown()
    held.layer?.let { bindings.remove(it) }
    if (Media.audio === audio) Media.audio = null
  }
  val releaser = object : MemoryReleaser {
    override fun dropMemoryCaches() {
      synchronized(lock) { imageLoader }?.memoryCache?.clear()
    }

    override suspend fun shutdown() {
      val held = detach()
      try {
        held.signals?.unregister()
      } finally {
        finish(held)
      }
    }
  }
  Media.audio = audio
  val created = AtomicReference<MediaLayer?>()
  val opened = openScope.async {
    val client by lazy(LazyThreadSafetyMode.NONE) { OkHttpClient() }
    Trace.beginSection("MediaLayer.open.platform")
    val slots: ExoPlayerSlotFactory
    val dir: File
    val availableBytes: ULong
    val device: DeviceMemory
    try {
      slots = ExoPlayerSlotFactory(app, callFactory ?: client, startMuted = silent, audio = audio, videoOutputCounters = videoOutputCounters)
      synchronized(lock) { factory = slots }
      dir = File(app.cacheDir, "medialayer").apply { mkdirs() }
      availableBytes = android.os.StatFs(dir.path).availableBytes.toULong()
      device = deviceMemory(app)
    } finally { Trace.endSection() }
    ensureActive()
    Trace.beginSection("MediaLayer.open.jna")
    try { install() } finally { Trace.endSection() }
    Trace.beginSection("MediaLayer.open.defaults")
    val resolved = try { defaults() } finally { Trace.endSection() }
    val config = OpenConfig(
      dir = dir.path,
      availableBytes = availableBytes,
      fetcher = fetcher ?: OkHttpFetcher(client),
      settings = settings,
      slots = slots,
      device = device,
      defaults = resolved.defaults,
      pool = resolved.pool,
      namespaceHeaders = resolved.namespaceHeaders,
      imageWindows = resolved.imageWindows,
      forwardHeaders = resolved.forwardHeaders,
      evictVideo = resolved.evictVideo,
      evictImage = resolved.evictImage,
      evictAudio = resolved.evictAudio,
      memoryReleaser = releaser,
      silent = silent
    )
    ensureActive()
    Trace.beginSection("MediaLayer.open.ffi")
    val layer = try { open(config) } finally { Trace.endSection() }
    created.set(layer)
    ensureActive()
    synchronized(lock) { reference = WeakReference(layer) }
    val loader = mediaImageLoader(app, ImageBytes { url, headers ->
      val media = reference?.get() ?: throw CancellationException("MediaLayer was closed")
      media.fetchImage(url, headers)
    }, imageMemoryBytes, stats = layer.imageStats())
    synchronized(lock) { imageLoader = loader }
    layer.setTonePreparation(ToneFrames(layer, app, loader))
    val events = layer.playerEvents()
    slots.bind(events)
    val registered = SystemSignals(app, events, lifecycle).also { it.register() }
    synchronized(lock) { signals = registered }
    bindings[layer] = Binding(slots, registered, loader)
    layer
  }
  opened.invokeOnCompletion { failure ->
    val layer = created.getAndSet(null)
    if (failure != null) {
      layer?.close()
      val held = detach()
      finish(held)
      held.signals?.let { openScope.launch { it.unregister() } }
    }
  }
  return opened
}

internal fun deviceMemory(context: Context): DeviceMemory {
  val manager = context.getSystemService(ActivityManager::class.java)
  val info = ActivityManager.MemoryInfo().also { manager.getMemoryInfo(it) }
  return DeviceMemory((info.totalMem / BYTES_PER_MB).toUInt(), manager.memoryClass.toUInt(), manager.isLowRamDevice)
}

private const val BYTES_PER_MB = 1024L * 1024L

@Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
internal class SystemSignals(
  private val context: Context,
  private val events: PlayerEventsInterface,
  private val lifecycle: Lifecycle
) : ComponentCallbacks2, DefaultLifecycleObserver {
  private val outputLost = object : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
      if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) Media.muted = true
    }
  }

  fun register() {
    context.registerComponentCallbacks(this)
    ContextCompat.registerReceiver(context, outputLost, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_NOT_EXPORTED)
    onMain { lifecycle.addObserver(this) }
  }

  suspend fun unregister() = withContext(Dispatchers.Main.immediate) {
    context.unregisterComponentCallbacks(this@SystemSignals)
    context.unregisterReceiver(outputLost)
    lifecycle.removeObserver(this@SystemSignals)
  }

  override fun onTrimMemory(level: Int) {
    pressure(level)?.let { events.memoryWarning(it) }
  }

  override fun onLowMemory() = events.memoryWarning(Pressure.CRITICAL)

  override fun onConfigurationChanged(newConfig: Configuration) = Unit

  override fun onStart(owner: LifecycleOwner) = events.foreground()

  override fun onStop(owner: LifecycleOwner) = events.background()

  private fun pressure(level: Int): Pressure? = when {
    level == ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN -> null
    level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL -> Pressure.CRITICAL
    level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW -> Pressure.MODERATE
    else -> null
  }

  private fun onMain(work: () -> Unit) {
    if (Looper.myLooper() == Looper.getMainLooper()) work() else Handler(Looper.getMainLooper()).post(work)
  }
}

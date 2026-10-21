package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import android.os.Trace
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
import kotlinx.coroutines.CancellationException
import coil.ImageLoader
import okhttp3.Call
import okhttp3.OkHttpClient

internal class Binding(val factory: ExoPlayerSlotFactory, val signals: SystemSignals, val imageLoader: ImageLoader)

private val bindings: MutableMap<MediaLayer, Binding> = Collections.synchronizedMap(WeakHashMap())

internal val MediaLayer.binding: Binding? get() = bindings[this]

fun MediaLayer.Companion.open(
  context: Context,
  fetcher: Fetcher? = null,
  settings: SettingsStore,
  callFactory: Call.Factory? = null,
  silent: Boolean = false,
  audioFocus: MediaAudioFocus = MediaAudioFocus.Mix,
  imageMemoryBytes: Int? = null
): MediaLayer =
  openMediaLayer(context, fetcher, settings, callFactory, silent = silent, lifecycle = ProcessLifecycleOwner.get().lifecycle, audioFocus = audioFocus, imageMemoryBytes = imageMemoryBytes) { MediaLayer.open(it) }

fun MediaLayer.Companion.open(
  context: Context,
  spec: String,
  fetcher: Fetcher? = null,
  settings: SettingsStore,
  callFactory: Call.Factory? = null,
  silent: Boolean = false,
  audioFocus: MediaAudioFocus = MediaAudioFocus.Mix,
  imageMemoryBytes: Int? = null
): MediaLayer =
  openMediaLayer(context, fetcher, settings, callFactory, defaults = { specDefaults(spec) }, silent = silent, lifecycle = ProcessLifecycleOwner.get().lifecycle, audioFocus = audioFocus, imageMemoryBytes = imageMemoryBytes) { MediaLayer.open(it) }

private fun platformDefaults(): SpecDefaults = SpecDefaults(defaultLimits(), defaultPoolSettings(), emptyList())

fun MediaLayer.release() {
  bindings.remove(this)?.let {
    it.signals.unregister()
    it.factory.release()
    if (Media.audio === it.factory.audio) Media.audio = null
  }
  close()
}

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
  open: (OpenConfig) -> MediaLayer
): MediaLayer {
  Trace.beginSection("MediaLayer.open.jna")
  try { NativeCallbacks.install() } finally { Trace.endSection() }
  Trace.beginSection("MediaLayer.open.defaults")
  val resolved = try { defaults() } finally { Trace.endSection() }
  Trace.beginSection("MediaLayer.open.platform")
  try {
    val app = context.applicationContext
    val client by lazy(LazyThreadSafetyMode.NONE) { OkHttpClient() }
    val audio = MediaAudioSession(app, audioFocus)
    val factory = ExoPlayerSlotFactory(app, callFactory ?: client, startMuted = silent, audio = audio)
    val dir = File(app.cacheDir, "medialayer").apply { mkdirs() }
    var reference: WeakReference<MediaLayer>? = null
    val imageLoader = mediaImageLoader(app, ImageBytes { url, headers ->
      val media = reference?.get() ?: throw CancellationException("MediaLayer was released")
      media.fetchImage(url, headers)
    }, imageMemoryBytes)
    val releaser = object : MemoryReleaser {
      override fun dropMemoryCaches() {
        imageLoader.memoryCache?.clear()
      }
    }
    val config = OpenConfig(
      dir = dir.path,
      fetcher = fetcher ?: OkHttpFetcher(client),
      settings = settings,
      slots = factory,
      device = deviceMemory(app),
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
    Trace.beginSection("MediaLayer.open.ffi")
    val layer = try { open(config) } finally { Trace.endSection() }
    reference = WeakReference(layer)
    val events = layer.playerEvents()
    factory.bind(events)
    val signals = SystemSignals(app, events, lifecycle).also { it.register() }
    bindings[layer] = Binding(factory, signals, imageLoader)
    Media.audio = audio
    return layer
  } finally { Trace.endSection() }
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
  fun register() {
    context.registerComponentCallbacks(this)
    onMain { lifecycle.addObserver(this) }
  }

  fun unregister() {
    context.unregisterComponentCallbacks(this)
    onMain { lifecycle.removeObserver(this) }
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

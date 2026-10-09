package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MemoryReleaser
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.PlayerEventsInterface
import com.noma4i.medialayer.Pressure
import com.noma4i.medialayer.SettingsStore
import com.noma4i.medialayer.SpecDefaults
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.specDefaults
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.util.Collections
import java.util.WeakHashMap
import coil.ImageLoader
import okhttp3.Call
import okhttp3.OkHttpClient

internal class Binding(val factory: ExoPlayerSlotFactory, val signals: SystemSignals, val imageLoader: ImageLoader)

private val bindings: MutableMap<MediaLayer, Binding> = Collections.synchronizedMap(WeakHashMap())

internal val MediaLayer.binding: Binding? get() = bindings[this]

fun MediaLayer.Companion.open(
  context: Context,
  fetcher: Fetcher,
  settings: SettingsStore,
  callFactory: Call.Factory = OkHttpClient(),
  silent: Boolean = false,
  imageMemoryBytes: Int = DEFAULT_IMAGE_MEMORY_BYTES
): MediaLayer =
  openMediaLayer(context, fetcher, settings, callFactory, platformDefaults(), silent, ProcessLifecycleOwner.get().lifecycle, imageMemoryBytes) { MediaLayer.open(it) }

fun MediaLayer.Companion.open(
  context: Context,
  spec: String,
  fetcher: Fetcher,
  settings: SettingsStore,
  callFactory: Call.Factory = OkHttpClient(),
  silent: Boolean = false,
  imageMemoryBytes: Int = DEFAULT_IMAGE_MEMORY_BYTES
): MediaLayer =
  openMediaLayer(context, fetcher, settings, callFactory, specDefaults(spec), silent, ProcessLifecycleOwner.get().lifecycle, imageMemoryBytes) { MediaLayer.open(it) }

internal suspend fun <Result> offCaller(work: () -> Result): Result = withContext(Dispatchers.Default) { work() }

suspend fun MediaLayer.Companion.openInBackground(
  context: Context,
  spec: String,
  fetcher: Fetcher,
  settings: SettingsStore,
  callFactory: Call.Factory = OkHttpClient(),
  silent: Boolean = false,
  imageMemoryBytes: Int = DEFAULT_IMAGE_MEMORY_BYTES
): MediaLayer = offCaller { MediaLayer.open(context, spec, fetcher, settings, callFactory, silent, imageMemoryBytes) }

private fun platformDefaults(): SpecDefaults = SpecDefaults(defaultLimits(), defaultPoolSettings(), emptyList())

fun MediaLayer.release() {
  bindings.remove(this)?.let {
    it.signals.unregister()
    it.factory.release()
  }
  close()
}

internal fun openMediaLayer(
  context: Context,
  fetcher: Fetcher,
  settings: SettingsStore,
  callFactory: Call.Factory = OkHttpClient(),
  defaults: SpecDefaults = platformDefaults(),
  silent: Boolean = false,
  lifecycle: Lifecycle,
  imageMemoryBytes: Int = DEFAULT_IMAGE_MEMORY_BYTES,
  open: (OpenConfig) -> MediaLayer
): MediaLayer {
  val app = context.applicationContext
  val factory = ExoPlayerSlotFactory(app, callFactory, startMuted = silent || isDebuggable(app))
  val dir = File(app.cacheDir, "medialayer").apply { mkdirs() }
  lateinit var layer: MediaLayer
  val imageLoader = mediaImageLoader(app, ImageBytes { url, headers -> layer.fetchImage(url, headers) }, imageMemoryBytes)
  val releaser = object : MemoryReleaser {
    override fun dropMemoryCaches() {
      imageLoader.memoryCache?.clear()
    }
  }
  layer = open(
    OpenConfig(
      dir = dir.path,
      fetcher = fetcher,
      settings = settings,
      slots = factory,
      device = deviceMemory(app),
      defaults = defaults.defaults,
      pool = defaults.pool,
      namespaceHeaders = defaults.namespaceHeaders,
      imageWindows = defaults.imageWindows,
      memoryReleaser = releaser,
      silent = silent
    )
  )
  val events = layer.playerEvents()
  factory.bind(events)
  val signals = SystemSignals(app, events, lifecycle).also { it.register() }
  bindings[layer] = Binding(factory, signals, imageLoader)
  return layer
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

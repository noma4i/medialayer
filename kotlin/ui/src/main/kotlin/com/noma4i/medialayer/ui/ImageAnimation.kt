package com.noma4i.medialayer.ui

import android.content.Context
import android.database.ContentObserver
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Animatable
import android.graphics.drawable.Animatable2
import android.graphics.drawable.Drawable
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf

enum class RestFrame { First, Last }

sealed interface Repeats {
  data object Forever : Repeats
  data class Times(val count: Int) : Repeats
}

/** How a [MediaImage] plays an animated image (GIF, WebP) and what it shows while it rests. */
data class ImageAnimation(
  val playing: Boolean = true,
  val repeats: Repeats = Repeats.Forever,
  val rest: RestFrame = RestFrame.First,
  val ignoresReduceMotion: Boolean = false
) {
  internal fun rests(reduceMotion: Boolean): Boolean = !playing || (reduceMotion && !ignoresReduceMotion)
}

/** The playback of one loaded animated image: the request sets the repeats, the end of the last repeat reaches [onFinished]. */
internal interface RepeatingAnimation : Animatable {
  fun onFinished(callback: () -> Unit)
}

internal fun Animatable.repeating(): RepeatingAnimation? = when (this) {
  is RepeatingAnimation -> this
  is AnimatedImageDrawable -> DrawableRepeats(this)
  else -> null
}

private class DrawableRepeats(private val drawable: AnimatedImageDrawable) : RepeatingAnimation {
  override fun onFinished(callback: () -> Unit) {
    drawable.clearAnimationCallbacks()
    drawable.registerAnimationCallback(object : Animatable2.AnimationCallback() {
      override fun onAnimationEnd(drawable: Drawable) = callback()
    })
  }

  override fun start() = drawable.start()
  override fun stop() = drawable.stop()
  override fun isRunning() = drawable.isRunning
}

internal fun animatorsDisabled(context: Context): Boolean =
  Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f

/** The system "remove animations" setting: one process-wide registration, shared by every [MediaImage]. */
internal object SystemReduceMotion {
  private val flag = mutableStateOf(false)
  private var observer: ContentObserver? = null
  private var owner: Context? = null
  val state: State<Boolean> get() = flag

  @Synchronized
  fun install(context: Context) {
    val app = context.applicationContext
    if (owner === app) return

    owner?.let { previous -> observer?.let { previous.contentResolver.unregisterContentObserver(it) } }
    val registered = object : ContentObserver(Handler(Looper.getMainLooper())) {
      override fun onChange(selfChange: Boolean) { flag.value = animatorsDisabled(app) }
    }
    app.contentResolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, registered)
    flag.value = animatorsDisabled(app)
    observer = registered
    owner = app
  }
}

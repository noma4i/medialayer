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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

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

/** The system "remove animations" setting, read once and followed while the caller is composed. */
@Composable
internal fun rememberReduceMotion(): State<Boolean> {
  val context = LocalContext.current
  val state = remember(context) { mutableStateOf(animatorsDisabled(context)) }
  DisposableEffect(context) {
    val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
      override fun onChange(selfChange: Boolean) { state.value = animatorsDisabled(context) }
    }
    context.contentResolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, observer)
    state.value = animatorsDisabled(context)
    onDispose { context.contentResolver.unregisterContentObserver(observer) }
  }
  return state
}

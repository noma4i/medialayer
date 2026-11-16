package com.noma4i.medialayer.ui

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.ContextWrapper
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import java.util.WeakHashMap

internal enum class ScreenHoldKind { SOUND, ALWAYS }

/** One process counter of holders. A `SOUND` holder counts while the global mute is off, an `ALWAYS` holder counts always. The system port is called inside the counter order when "any counted holder" flips. */
internal class ScreenAwake(muted: Boolean = false, private val apply: (Boolean) -> Unit) {
  private val lock = Any()
  private val holders = HashMap<Any, ScreenHoldKind>()
  private var muted = muted

  private val counted: Boolean get() = holders.values.any { it == ScreenHoldKind.ALWAYS || !muted }

  private fun change(edit: () -> Unit) = synchronized(lock) {
    val before = counted
    edit()
    val after = counted
    if (before != after) apply(after)
  }

  fun set(holder: Any, kind: ScreenHoldKind, on: Boolean) = change { if (on) holders[holder] = kind else holders.remove(holder) }

  fun setMuted(value: Boolean) = change { muted = value }

  companion object {
    val shared = ScreenAwake(Media.muted, ScreenWindows::apply)
  }
}

/** One hold of one owner: a place (`SOUND`: while it plays and is not muted), a voice recorder, a camera or a call (`ALWAYS`). */
internal class ScreenAwakeHold(private val kind: ScreenHoldKind = ScreenHoldKind.ALWAYS, private val awake: ScreenAwake = ScreenAwake.shared) {
  private var playing = false
  private var muted = false

  fun set(on: Boolean) = update { playing = on }

  fun mute(value: Boolean) = update { muted = value }

  private fun update(edit: () -> Unit) = synchronized(this) {
    edit()
    awake.set(this, kind, playing && !muted)
  }
}

/** The system port: FLAG_KEEP_SCREEN_ON on the windows of the started activities it knows, applied on Main in the order of the flips. */
internal object ScreenWindows {
  private val main = Handler(Looper.getMainLooper())
  private val activities = WeakHashMap<Activity, Boolean>()
  private val lock = Any()
  private var registered = false
  private var on = false

  fun know(context: Context) {
    val app = context.applicationContext as? Application
    synchronized(lock) {
      if (!registered && app != null) {
        registered = true
        app.registerActivityLifecycleCallbacks(Callbacks)
      }
    }
    var current: Context? = context
    while (current is ContextWrapper) {
      if (current is Activity) { val activity = current; main.post { add(activity) }; return }
      current = current.baseContext
    }
  }

  fun apply(value: Boolean) {
    main.post {
      on = value
      activities.keys.toList().forEach(::paint)
    }
  }

  private fun add(activity: Activity) {
    if (activity.isFinishing || activity.isDestroyed) return
    activities[activity] = true
    paint(activity)
  }

  private fun paint(activity: Activity) {
    if (on) activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
  }

  private object Callbacks : Application.ActivityLifecycleCallbacks {
    override fun onActivityStarted(activity: Activity) = add(activity)
    override fun onActivityStopped(activity: Activity) { activities.remove(activity) }
    override fun onActivityDestroyed(activity: Activity) { activities.remove(activity) }
    override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
    override fun onActivityResumed(activity: Activity) = Unit
    override fun onActivityPaused(activity: Activity) = Unit
    override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
  }
}

internal fun knowing(context: Context): Context {
  ScreenWindows.know(context)
  return context.applicationContext
}

package com.noma4i.medialayer.ui

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.View
import android.view.Window
import android.view.WindowManager
import androidx.compose.ui.window.DialogWindowProvider
import java.util.WeakHashMap

/** Main-thread only. FLAG_KEEP_SCREEN_ON stays set on a window while at least one place of that window plays. */
internal object ScreenAwake {
  private val holders = WeakHashMap<Window, MutableSet<Any>>()

  fun set(window: Window, holder: Any, playing: Boolean) {
    val set = holders.getOrPut(window) { mutableSetOf() }
    val before = set.isNotEmpty()
    if (playing) set.add(holder) else set.remove(holder)
    val after = set.isNotEmpty()
    if (before == after) return
    if (after) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
  }

  fun window(view: View): Window? {
    (view.parent as? DialogWindowProvider)?.let { return it.window }
    var context: Context? = view.context
    while (context is ContextWrapper) {
      if (context is Activity) return context.window
      context = context.baseContext
    }
    return null
  }
}

internal class ScreenAwakeHold(private val view: View) {
  private var window: Window? = null
  fun set(playing: Boolean) {
    val target = if (playing) ScreenAwake.window(view) else window
    if (target == null) return
    if (window != null && window !== target) ScreenAwake.set(window!!, this, false)
    ScreenAwake.set(target, this, playing)
    window = if (playing) target else null
  }
}

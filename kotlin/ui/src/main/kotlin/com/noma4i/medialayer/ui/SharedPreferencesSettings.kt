package com.noma4i.medialayer.ui

import android.content.Context
import com.noma4i.medialayer.SettingsStore

class SharedPreferencesSettings(context: Context, name: String = "medialayer") : SettingsStore {
  private val prefs = context.applicationContext.getSharedPreferences(name, Context.MODE_PRIVATE)

  override fun read(key: String): String? = prefs.getString(key, null)
  override fun write(key: String, value: String) = prefs.edit().putString(key, value).apply()
  override fun remove(key: String) = prefs.edit().remove(key).apply()
}

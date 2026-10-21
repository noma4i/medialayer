package com.noma4i.medialayer.ui

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class SharedPreferencesSettingsTest {
  @Test
  fun valuesSurviveANewInstanceAndRemoveDeletesThem() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    SharedPreferencesSettings(context).write("medialayer.limits", "{}")

    val reopened = SharedPreferencesSettings(context)
    assertEquals("{}", reopened.read("medialayer.limits"))
    reopened.remove("medialayer.limits")

    assertNull(SharedPreferencesSettings(context).read("medialayer.limits"))
  }
}

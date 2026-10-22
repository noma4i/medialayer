package com.noma4i.medialayer.ui

import com.noma4i.medialayer.ImageCounters
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.JUnitCore
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

class CallbackClassLoaderTest {
  @Test(timeout = 120_000)
  fun inv_ffi_10_callbacks_survive_sdk_and_plain_jvm_tests() {
    val result = JUnitCore.runClasses(SdkCalls::class.java, JvmCalls::class.java, OpenPolicyTest::class.java)
    assertTrue(result.failures.joinToString("\n") { it.trace }, result.wasSuccessful())
  }

  @RunWith(MediaLayerTestRunner::class)
  @Config(sdk = [33, 34])
  class SdkCalls {
    @Test
    fun initializeNativeCallbacks() {
      ImageCounters().use { it.snapshot() }
    }
  }

  class JvmCalls {
    @Test
    fun initializeNativeCallbacks() {
      ImageCounters().use { it.snapshot() }
    }
  }
}

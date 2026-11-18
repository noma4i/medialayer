package com.noma4i.medialayer.ui

import com.google.common.reflect.ClassPath
import org.junit.runners.model.FrameworkMethod
import org.objectweb.asm.ClassReader
import org.objectweb.asm.ClassVisitor
import org.objectweb.asm.Opcodes
import org.robolectric.RobolectricTestRunner
import org.robolectric.DefaultTestLifecycle
import org.robolectric.TestLifecycle
import org.robolectric.internal.bytecode.InstrumentationConfiguration

class MediaLayerTestRunner(testClass: Class<*>) : RobolectricTestRunner(testClass) {
  override fun getTestLifecycleClass(): Class<out TestLifecycle> = MediaLayerTestLifecycle::class.java

  override fun createClassLoaderConfig(method: FrameworkMethod): InstrumentationConfiguration =
    InstrumentationConfiguration.Builder(super.createClassLoaderConfig(method)).apply {
      bridgeClasses.forEach { doNotAcquireClass(it) }
    }.build()

  companion object {
    private val bridgeClasses = ClassPath.from(MediaLayerTestRunner::class.java.classLoader)
      .allClasses.filter { it.packageName == "com.noma4i.medialayer" }.filter { candidate ->
        var generated = false
        candidate.asByteSource().openStream().use { bytes ->
          ClassReader(bytes).accept(object : ClassVisitor(Opcodes.ASM9) {
            override fun visitSource(source: String?, debug: String?) {
              generated = source == "medialayer.kt"
            }
          }, ClassReader.SKIP_CODE or ClassReader.SKIP_FRAMES)
        }
        generated
      }.map { it.name }.also {
        check(it.isNotEmpty()) { "No classes found for generated UniFFI SourceFile medialayer.kt" }
      }
  }
}

class MediaLayerTestLifecycle : DefaultTestLifecycle() {
  override fun afterTest(method: java.lang.reflect.Method) {
    super.afterTest(method)
    ProcessMediaEnvironment.reset()
    ScreenAwake.shared.reset()
    ScreenWindows.reset()
    org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
  }
}

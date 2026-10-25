plugins {
  id("com.android.library") version "9.4.1"
}

android {
  namespace = "com.noma4i.medialayer.testing"
  compileSdk = 37

  defaultConfig {
    minSdk = 33
  }

  testOptions {
    unitTests.all {
      it.doFirst {
        val hostLibrary = file("../host-library.path")
        if (!hostLibrary.exists()) throw GradleException("host-library.path is missing: run scripts/build_kotlin.sh --host")
        it.systemProperty("jna.library.path", hostLibrary.readText().trim())
      }
    }
  }
}

dependencies {
  api(project(":"))
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
  implementation("net.java.dev.jna:jna:5.19.1@aar")

  testImplementation("net.java.dev.jna:jna:5.19.1")
  testImplementation("junit:junit:4.13.2")
}

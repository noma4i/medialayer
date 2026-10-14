plugins {
  id("com.android.library") version "9.4.1"
  id("org.jetbrains.kotlin.plugin.compose") version "2.3.21"
}

android {
  namespace = "com.noma4i.medialayer.ui"
  compileSdk = 37

  defaultConfig {
    minSdk = 26
  }

  buildFeatures {
    compose = true
  }

  testOptions {
    unitTests.isIncludeAndroidResources = true
    unitTests.all {
      it.systemProperty("jna.library.path", file("../../target/debug").absolutePath)
    }
  }
}

dependencies {
  api(project(":"))
  implementation(platform("androidx.compose:compose-bom:2026.09.00"))
  implementation("androidx.compose.ui:ui")
  implementation("androidx.compose.foundation:foundation")
  implementation("androidx.compose.runtime:runtime")
  testImplementation("androidx.compose.material3:material3")
  implementation("androidx.media3:media3-exoplayer:1.11.1")
  implementation("androidx.media3:media3-exoplayer-hls:1.11.1")
  implementation("androidx.media3:media3-datasource-okhttp:1.11.1")
  implementation("androidx.media3:media3-ui-compose:1.11.1")
  implementation("androidx.lifecycle:lifecycle-process:2.10.0")
  implementation("com.squareup.okhttp3:okhttp:5.5.0")
  implementation("io.coil-kt:coil-compose:2.7.0")
  implementation("io.coil-kt:coil-gif:2.7.0")
  implementation("net.java.dev.jna:jna:5.17.0@aar")

  testImplementation("net.java.dev.jna:jna:5.17.0")
  testImplementation("junit:junit:4.13.2")
  testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
  testImplementation("org.robolectric:robolectric:4.17")
  testImplementation("androidx.test:core:1.7.0")
  testImplementation("androidx.media3:media3-test-utils:1.11.1")
  testImplementation("androidx.media3:media3-test-utils-robolectric:1.11.1")
  testImplementation("androidx.compose.ui:ui-test-junit4")
  debugImplementation("androidx.compose.ui:ui-test-manifest")
}

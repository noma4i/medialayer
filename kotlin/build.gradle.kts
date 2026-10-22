plugins {
  id("com.android.library") version "9.4.1"
}

android {
  namespace = "com.noma4i.medialayer"
  compileSdk = 37

  defaultConfig {
    minSdk = 26
    consumerProguardFiles("consumer-rules.pro")
    ndk {
      abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64", "x86")
    }
  }

  testOptions {
    unitTests.all {
      it.systemProperty("jna.library.path", file("host-library.path").readText().trim())
    }
  }
}

dependencies {
  implementation("net.java.dev.jna:jna:5.17.0@aar")
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
  testImplementation("net.java.dev.jna:jna:5.17.0")
  testImplementation("junit:junit:4.13.2")
}

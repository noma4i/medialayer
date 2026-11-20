plugins {
  id("com.android.library") version "9.4.1"
  id("org.jetbrains.kotlin.plugin.compose") version "2.4.21"
  id("maven-publish")
}

group = "com.noma4i"

android {
  namespace = "com.noma4i.medialayer.ui"
  compileSdk = 37

  defaultConfig {
    minSdk = 33
    testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
  }

  publishing {
    singleVariant("release")
  }

  buildFeatures {
    compose = true
  }

  testOptions {
    unitTests.isIncludeAndroidResources = true
    unitTests.all {
      it.addTestListener(object : TestListener {
        override fun beforeSuite(suite: TestDescriptor) = Unit
        override fun afterSuite(suite: TestDescriptor, result: TestResult) = Unit
        override fun beforeTest(test: TestDescriptor) {
          println("ORDER ${System.currentTimeMillis()} START 0 ${test.className}.${test.name}")
        }
        override fun afterTest(test: TestDescriptor, result: TestResult) {
          println("ORDER ${System.currentTimeMillis()} ${result.resultType} ${result.endTime - result.startTime} ${test.className}.${test.name}")
        }
      })
      it.doFirst {
        val hostLibrary = file("../host-library.path")
        if (!hostLibrary.exists()) throw GradleException("host-library.path is missing: run scripts/build_kotlin.sh --host")
        it.systemProperty("jna.library.path", hostLibrary.readText().trim())
      }
    }
  }
}

if (providers.gradleProperty("releaseVersion").isPresent && providers.gradleProperty("stagingRepo").isPresent) {
  afterEvaluate {
    publishing {
      publications {
        register<MavenPublication>("release") {
          groupId = "com.noma4i"
          artifactId = "medialayer-ui"
          version = providers.gradleProperty("releaseVersion").get()
          from(components["release"])
        }
      }
      repositories {
        maven {
          name = "staging"
          url = uri(providers.gradleProperty("stagingRepo").get())
        }
      }
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
  implementation("androidx.media3:media3-transformer:1.11.1")
  implementation("androidx.media3:media3-effect:1.11.1")
  implementation("androidx.media3:media3-inspector:1.11.1")
  implementation("androidx.media3:media3-datasource-okhttp:1.11.1")
  implementation("androidx.media3:media3-ui-compose:1.11.1")
  implementation("androidx.core:core-ktx:1.19.1")
  api("androidx.core:core-telecom:1.1.0")
  implementation("androidx.lifecycle:lifecycle-process:2.10.0")
  implementation("androidx.camera:camera-camera2:1.6.2")
  implementation("androidx.camera:camera-lifecycle:1.6.2")
  implementation("androidx.camera:camera-video:1.6.2")
  implementation("androidx.camera:camera-view:1.6.2")
  implementation("com.squareup.okhttp3:okhttp:5.5.0")
  implementation("io.coil-kt.coil3:coil-compose:3.6.3")
  implementation("io.coil-kt.coil3:coil-gif:3.6.3")
  implementation("net.java.dev.jna:jna:5.19.1@aar")

  testImplementation("net.java.dev.jna:jna:5.19.1")
  testImplementation("junit:junit:4.13.2")
  testImplementation(project(":testing"))
  testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
  testImplementation("org.robolectric:robolectric:4.17")
  testImplementation("androidx.test:core:1.7.0")
  testImplementation("androidx.media3:media3-test-utils:1.11.1")
  testImplementation("androidx.media3:media3-test-utils-robolectric:1.11.1")
  testImplementation("androidx.compose.ui:ui-test-junit4")
  debugImplementation("androidx.compose.ui:ui-test-manifest")
  androidTestImplementation("androidx.test:core:1.7.0")
  androidTestImplementation("androidx.test:runner:1.7.0")
  androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

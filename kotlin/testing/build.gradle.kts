plugins {
  id("com.android.library") version "9.4.1"
  id("maven-publish")
}

group = "com.noma4i"

android {
  namespace = "com.noma4i.medialayer.testing"
  compileSdk = 37

  defaultConfig {
    minSdk = 33
  }

  publishing {
    singleVariant("release")
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

if (providers.gradleProperty("releaseVersion").isPresent && providers.gradleProperty("stagingRepo").isPresent) {
  afterEvaluate {
    publishing {
      publications {
        register<MavenPublication>("release") {
          groupId = "com.noma4i"
          artifactId = "medialayer-testing"
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
  implementation(project(":ui"))
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
  implementation("net.java.dev.jna:jna:5.19.1@aar")

  testImplementation("net.java.dev.jna:jna:5.19.1")
  testImplementation("junit:junit:4.13.2")
}

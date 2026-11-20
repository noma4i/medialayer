buildscript {
  repositories {
    google()
    mavenCentral()
  }
  dependencies {
    classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.21")
    classpath("org.jetbrains.kotlinx:binary-compatibility-validator:0.18.2")
  }
}

plugins {
  id("com.android.library") version "9.4.1"
  id("maven-publish")
}

group = "com.noma4i"

allprojects {
  val apiSchemaRuntime = configurations.create("apiSchemaRuntime")
  dependencies.add(apiSchemaRuntime.name, "org.ow2.asm:asm:9.6")
  dependencies.add(apiSchemaRuntime.name, "org.ow2.asm:asm-tree:9.6")
  dependencies.add(apiSchemaRuntime.name, "org.jetbrains.kotlin:kotlin-metadata-jvm:2.4.21")
  tasks.register<kotlinx.validation.KotlinApiBuildTask>("dumpPublicApi") {
    val compile = tasks.named("compileReleaseKotlin")
    dependsOn(compile)
    inputClassesDirs.from(compile.map { it.outputs.files })
    runtimeClasspath.from(apiSchemaRuntime)
    outputApiFile.set(layout.buildDirectory.file("api/public.api"))
  }
}

android {
  namespace = "com.noma4i.medialayer"
  compileSdk = 37

  defaultConfig {
    minSdk = 33
    consumerProguardFiles("consumer-rules.pro")
    ndk {
      abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64", "x86")
    }
  }

  publishing {
    singleVariant("release")
  }

  testOptions {
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
        val hostLibrary = file("host-library.path")
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
          artifactId = "medialayer"
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
  implementation("net.java.dev.jna:jna:5.19.1@aar")
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
  testImplementation("net.java.dev.jna:jna:5.19.1")
  testImplementation("junit:junit:4.13.2")
  testImplementation(project(":testing"))
}

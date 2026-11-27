# Getting started

## Use a release

`.package(url: "https://github.com/noma4i/medialayer.git", exact: "X.Y.Z")`

Gradle repository: `repositories { maven { url = uri("https://raw.githubusercontent.com/noma4i/medialayer/maven") } }`

Gradle dependencies: `implementation("com.noma4i:medialayer:X.Y.Z")` and `implementation("com.noma4i:medialayer-ui:X.Y.Z")`.

| File | What | Where |
| --- | --- | --- |
| `MediaLayerFFI.xcframework.zip` | iOS device and simulator libraries | SwiftPM binary target |
| `medialayer.swift` | Generated Swift bridge | Tagged source at `MediaLayer/Sources/MediaLayer/medialayer.swift` |
| `medialayer-android.zip` | Android libraries for four ABIs and generated Kotlin bindings | Unzip into `kotlin/` |
| `medialayer-gen-macos-universal` | Generator for macOS arm64 and x86_64 | Run as `medialayer-gen` |

## Requirements

| Tool | Why |
| ---- | --- |
| Xcode (iOS 17+, macOS 14+) | Swift package and iOS simulator build |
| Homebrew `rustup` with the iOS and Android targets | Rust core for every platform |
| `cargo-ndk` and Android NDK | Android libraries |
| JDK 17 and Gradle 9, on brick only | Kotlin module |

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
bash scripts/check_medialayer.sh
```

On macOS the Kotlin lane and every Gradle step stop at once with `gradle runs only on brick` and a nonzero code. Run them on brick. `scripts/build_kotlin.sh` starts no Gradle and runs on any host.

## Build modes

| Command | Output |
| ------- | ------ |
| `bash scripts/build_ios.sh --device` | Release XCFramework containing only `aarch64-apple-ios`, for App Store archives |
| `bash scripts/build_ios.sh --ios` | Release iOS device and arm64 simulator slices |
| `bash scripts/build_ios.sh --check` | Release macOS arm64 and iOS arm64 simulator slices |
| `bash scripts/build_ios.sh` | Release macOS arm64, iOS device and arm64 simulator slices |
| `bash scripts/build_kotlin.sh --release` | Release `arm64-v8a` and `armeabi-v7a` libraries and Kotlin bindings, for Android AAB builds |
| `bash scripts/build_kotlin.sh --abis` | All 4 Android ABIs, including `x86` and `x86_64`, without regenerating bindings |
| `bash scripts/build_kotlin.sh --host` | Linux debug library and Kotlin bindings for JNA tests |
| `bash scripts/build_kotlin.sh` | Linux debug library, Kotlin bindings and all 4 Android ABIs |

iOS bindings come from a selected release static library. Android release bindings come from the `arm64-v8a` library.
The separate `bindgen` package builds `uniffi-bindgen` without building a host MediaLayer library.
Android `--release` removes previous `x86` and `x86_64` outputs from `jniLibs`.
On Linux, install `cargo-ndk` and the Rust Android targets. Set `ANDROID_NDK_HOME` when the NDK is outside the SDK location detected by `cargo-ndk`.

## Set up in your app

1. Describe your media in a `.mdl` file ([media files](media-files.md)).
2. Open MediaLayer once at launch with your network access and local storage.
3. Declare places with `MediaVideo`, `MediaImage` and `MediaAudio` ([declaring media](declaring-media.md)).

### Swift

```swift
@main
struct ExampleApp: App {
  private let opening = Task { try await MediaLayer.open() }

  var body: some Scene {
    WindowGroup {
      RootView().mediaLayer(opening: opening)
    }
  }
}
```

### Kotlin

```kotlin
class ExampleApp : Application() {
  lateinit var media: Deferred<MediaLayer>

  override fun onCreate() {
    super.onCreate()
    media = MediaLayer.open(context = this)
  }
}
```

### Setup without readiness code

The app does not write a readiness state machine for `MediaLayer`. Four rules:

- `open` defaults: Swift takes `URLSessionFetcher()` and `UserDefaultsSettings()`, Kotlin takes `OkHttpFetcher` and `SharedPreferencesSettings(context)`. An explicit `fetcher` or `settings` replaces the default.
- Swift `.mediaLayer(opening: Task<MediaLayer, Error>)` draws the content at once. Media views do not declare places until `MediaLayer` is open, then they mount by themselves. An open error shows in `Media.failure`. Kotlin `MediaLayerProvider(Deferred)` works the same way and writes any open error to `Media.failure` (an error that is not a `MediaException` arrives as `MediaException.Unexpected`).
- Process context: a view without `MediaLayerProvider` in its tree (a root outside the tree, a toast, a window) takes the context of the last installed `MediaLayerProvider` of the process. The environment context wins over the process context. Removing `MediaLayerProvider` removes the process context if it is the current one. Without both contexts Kotlin throws "Wrap the screen in MediaLayerProvider" and Swift draws no media.
- `MediaLayer` from UI: Swift `@Environment(\.mediaLayer)` and Kotlin `currentMediaLayer()` return `MediaLayer?`. `nil` means `MediaLayer` is not ready yet. Both use the process context.

Open returns without waiting for the core assembly. Swift `open` is `async`, Kotlin `open` returns `Deferred<MediaLayer>`. Loading the native library, parsing `spec` and assembling the core run off the main thread. Android creates the platform objects and the cache directory on the calling thread. `MediaLayerProvider` takes the `Deferred` and shows the content when `MediaLayer` is ready. Pass `spec` for the values from `.mdl` ([media files](media-files.md)).

Android creates one shared `OkHttpClient` for the default fetcher and the players. An explicit `fetcher` and `callFactory` replace the matching dependency.

MediaLayer plays remote video through its local proxy at `http://127.0.0.1:<port>`. The network security config of the app must allow cleartext traffic to `127.0.0.1` in every build type, for example `<domain-config cleartextTrafficPermitted="true"><domain includeSubdomains="false">127.0.0.1</domain></domain-config>`. Without it, OkHttp refuses the cleartext request and every remote video fails.

## Open with full control

The calls above fill `OpenConfig` for you. Pass it yourself to change the pool, the device class or the cache key.

The cache budget is `Limits.budget` as configured. Free disk space never changes it. When the disk cannot store a response, the bytes are still delivered and the store failure is reported on the failure stream (`docs/failures.md`).

### Swift

```swift
let media = try MediaLayer.open(config: OpenConfig(
  dir: cacheDirectory.path,
  fetcher: URLSessionFetcher(),
  settings: UserDefaultsSettings(),
  slots: AVQueuePlayerSlotFactory(startMuted: false),
  device: DeviceMemory(totalMb: 6144, heapClassMb: nil, lowRam: false),
  defaults: defaultLimits(),
  pool: defaultPoolSettings(),
  namespaceHeaders: ["Authorization"],
  memoryReleaser: nil,
  silent: false,
  audioFocus: .exclusive
))
```

### Kotlin

```kotlin
val media = MediaLayer.open(OpenConfig(
  dir = cacheDir.path,
  fetcher = OkHttpFetcher(okHttpClient),
  settings = SharedPreferencesSettings(context),
  slots = ExoPlayerSlotFactory(context),
  device = DeviceMemory(totalMb = 3072u, heapClassMb = 256u, lowRam = false),
  defaults = defaultLimits(),
  pool = defaultPoolSettings(),
  namespaceHeaders = listOf("Authorization"),
  memoryReleaser = null,
  silent = false,
  audioFocus = AudioFocus.EXCLUSIVE
))
```

## Where to go next

- [Media files](media-files.md)
- [Declaring media](declaring-media.md)
- [Player pool](player-pool.md)
- [Placeholders](placeholders.md)
- [Limits and eviction](limits-and-eviction.md)
- [Memory control](memory.md)
- [Threading and performance](threading.md)

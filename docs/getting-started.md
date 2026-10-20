# Getting started

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

On macOS the Kotlin lane, `scripts/build_kotlin.sh` and every Gradle step stop at once with `gradle runs only on brick` and a nonzero code. Run them on brick.

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
struct YupiApp: App {
  let media = try! MediaLayer.open(
    fetcher: URLSessionFetcher(),
    settings: UserDefaultsSettings()
  )

  var body: some Scene {
    WindowGroup { RootView().mediaLayer(media) }
  }
}
```

### Kotlin

```kotlin
class YupiApp : Application() {
  lateinit var media: MediaLayer

  override fun onCreate() {
    super.onCreate()
    media = MediaLayer.open(
      context = this,
      fetcher = OkHttpFetcher(okHttpClient),
      settings = SharedPreferencesSettings(this)
    )
  }
}
```

Opening returns at once. Nothing is read from disk until it is needed. To take the defaults from your `.mdl` file, pass `spec` ([media files](media-files.md)).

## Open with full control

The calls above fill `OpenConfig` for you. Pass it yourself to change the pool, the device class or the cache key.

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
  silent: false
))
```

### Kotlin

```kotlin
val media = MediaLayer.open(OpenConfig(
  dir = cacheDir.path,
  fetcher = OkHttpFetcher(okHttpClient),
  settings = SharedPreferencesSettings(this),
  slots = ExoPlayerSlotFactory(this),
  device = DeviceMemory(totalMb = 3072u, heapClassMb = 256u, lowRam = false),
  defaults = defaultLimits(),
  pool = defaultPoolSettings(),
  namespaceHeaders = listOf("Authorization"),
  memoryReleaser = null,
  silent = false
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

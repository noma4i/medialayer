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

### Подключение без кода готовности

Приложение не пишет машину готовности слоя. Четыре правила:

- Умолчания `open`: Swift берёт `URLSessionFetcher()` и `UserDefaultsSettings()`, Kotlin берёт `OkHttpFetcher` и `SharedPreferencesSettings(context)`. Явный `fetcher` или `settings` заменяет умолчание.
- Swift `.mediaLayer(opening: Task<MediaLayer, Error>)` рисует содержимое сразу. Виды медиа не объявляют места, пока слой не открыт, потом монтируются сами. Ошибка открытия видна в `Media.failure`. Kotlin `MediaLayerProvider(Deferred)` ведёт себя так же и пишет любую ошибку открытия в `Media.failure` (не `MediaException` приходит как `MediaException.Unexpected`).
- Процессный контекст: вид без провайдера в дереве (корень вне дерева, тост, окно) берёт контекст последнего установленного провайдера процесса. Контекст окружения сильнее процессного. Снятие провайдера снимает процессный контекст, если он текущий. Без обоих контекстов Kotlin бросает ошибку «Wrap the screen in MediaLayerProvider», Swift не рисует медиа.
- Слой из UI: Swift `@Environment(\.mediaLayer)` и Kotlin `currentMediaLayer()` отдают `MediaLayer?`. Значение `nil` значит, что слой ещё не готов. Оба учитывают процессный контекст.

Открытие возвращается без ожидания сборки ядра. Swift `open` это `async`, Kotlin `open` возвращает `Deferred<MediaLayer>`. Загрузка нативной библиотеки, разбор `spec` и сборка ядра идут вне главного потока. Android создаёт платформенные объекты и каталог кэша на потоке вызова. `MediaLayerProvider` принимает `Deferred` и показывает содержимое, когда слой готов. Передайте `spec` для значений из `.mdl` ([media files](media-files.md)).

Android создаёт общий `OkHttpClient` для стандартного fetcher и плееров. Явные `fetcher` и `callFactory` заменяют соответствующую зависимость.

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

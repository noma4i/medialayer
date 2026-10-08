# Getting started

## Requirements

| Tool | Why |
| ---- | --- |
| Xcode (iOS 17+, macOS 14+) | Swift package and iOS simulator build |
| Homebrew `rustup` with the iOS and Android targets | Rust core for every platform |
| `cargo-ndk` and Android NDK | Android libraries |
| JDK 17 and Gradle 9 | Kotlin module |

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
bash scripts/check_medialayer.sh
```

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
      fetcher = OkHttpFetcher(okHttpClient),
      settings = SharedPreferencesSettings(this)
    )
  }
}
```

Opening returns at once. Nothing is read from disk until it is needed.

## Where to go next

- [Media files](media-files.md)
- [Declaring media](declaring-media.md)
- [Player pool](player-pool.md)
- [Placeholders](placeholders.md)
- [Limits and eviction](limits-and-eviction.md)
- [Memory control](memory.md)
- [Threading and performance](threading.md)

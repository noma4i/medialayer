# Memory control

Memory is managed automatically. You do not call anything: there is no pressure callback to forward and no profile to configure. `open` subscribes to the system signals itself, reads the device class, and the library frees memory before the system has to kill your app. It is built for low-end Android devices.

| What happens | When |
| ------------ | ---- |
| Player slots are reused, never created per screen | Always. An unloaded slot holds no frames, buffers or decoders |
| Slots drop from hot to warm to cold | After a slot sits idle for `trim_delay` (default 30 seconds) |
| Prefetch stops, then far slots are released, then the playing slot last | The system reports memory pressure (iOS memory warning, Android `onTrimMemory` and `onLowMemory`) |
| Everything except the chosen slot is released, prefetch stops | The app goes to the background |
| Slots return one by one | The app returns to the foreground |
| Fewer slots (at least 2), shorter buffer, lower bitrate, fewer parallel loads | The device is low-memory: Android `isLowRamDevice` or little RAM, iOS little physical memory |
| Everything stays under one limit | The `memory` option (default 64 MiB) covers players, proxy buffers and image memory |

Retention levels: `hot` plays or is next, `warm` keeps metadata and a small buffer, `cold` holds nothing.

## Opening is enough

### Swift

```swift
let media = try MediaLayer.open(
  fetcher: URLSessionFetcher(),
  settings: UserDefaultsSettings()
)
```

### Kotlin

```kotlin
val media = MediaLayer.open(
  context = applicationContext,
  fetcher = OkHttpFetcher(okHttpClient),
  settings = SharedPreferencesSettings(applicationContext)
)
```

On Android the context is how the library registers for `onTrimMemory`, `onLowMemory` and the app lifecycle. On iOS it observes the memory warning and the app lifecycle notifications.

## Tuning

Defaults are enough for most apps. To change them, set the options in your `.mdl` file.

```ruby
cache do
  memory 128.mib
end

video :moment_video do
  pool slots: 3, reach: 1, trim_delay: 30.seconds
end
```

## Looking at memory

`stats()` reports slots per level, estimated memory and the number of releases with their reason (timer, pressure, background, source change).

### Swift

```swift
let stats = await media.stats()
print(stats.slots.hot, stats.slots.warm, stats.slots.cold, stats.memoryBytes)
```

### Kotlin

```kotlin
val stats = media.stats()
println("${stats.slots.hot} ${stats.slots.warm} ${stats.slots.cold} ${stats.memoryBytes}")
```

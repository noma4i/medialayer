# Memory control

Memory is managed automatically. You do not call anything: there is no pressure callback to forward and no profile to configure. `open` subscribes to the system signals itself, reads the device class, and the library frees memory before the system has to kill your app. It is built for low-end Android devices.

| What happens | When |
| ------------ | ---- |
| Player slots are reused, never created per screen | Always. An unloaded slot holds no frames, buffers or decoders |
| Slots drop from hot to warm to cold | After a slot sits idle for `trim_delay` (default 30 seconds) |
| Prefetch stops, then far slots are released, then the playing slot last | The system reports memory pressure (iOS memory warning, Android `onTrimMemory` and `onLowMemory`) |
| Everything except the chosen slot is released, prefetch stops | The app goes to the background |
| Slots return one by one, the local proxy checks its own port and binds it again if the system closed it while the app slept | The app returns to the foreground |
| Fewer slots (at least 2), fewer chunks ahead of the playing position | The device is low-memory: Android `isLowRamDevice` or little RAM, iOS little physical memory |
| Everything stays under one limit | `memory` учитывает плееры и буферы прокси. Кэш картинок имеет отдельный бюджет: Android `min(128 MiB, 25% memoryClass)`, Swift 128 MiB |

## It cannot run the device out of memory

Every buffer, player slot, decode and queue reserves room in one counter before it allocates. If there is no room, the library first frees what is least needed. If that is not enough, it refuses the allocation: a place keeps its placeholder, a load waits, a prefetch is cancelled. Nothing is allocated past the limit, and a refused allocation never crashes the app.

The limit is the smaller of your `memory` option and a share of what the device really has (the Android heap class, the iOS physical memory). A low-memory device therefore cannot be pushed past its share by a large setting. Under growing pressure the library steps down in order: lower quality, fewer chunks ahead, fewer slots, only the playing slot, only placeholders. Images are decoded at the `size` from your media file or, without it, at the size of their view, never at original size.

Retention levels: `hot` plays or is next, `warm` keeps metadata, `cold` holds nothing.

UNBUILT: the steps lower the slot count, the reach, the admission share and the chunks ahead of the playing position (down to 1) today. A lower bitrate does not exist. Image decoding in Nuke and Coil does not reserve room in the counter; its memory is limited by `imageMemoryBytes`. A low-memory device gets fewer slots and a shorter reach, not fewer parallel loads.

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
  settings = SharedPreferencesSettings(applicationContext)
)
```

Android сохраняет `imageMemoryBytes` как явное положительное переопределение. Без него библиотека использует `ActivityManager.memoryClass`, включая устройства с `largeHeap`.
Бюджет ограничивает сохранённые bitmap в Coil, но не сумму одновременных декодов и памяти ядра.

On Android the context is how the library registers for `onTrimMemory`, `onLowMemory` and the app lifecycle. On iOS it observes the memory warning and the app lifecycle notifications.

## Tuning

Defaults are enough for most apps. To change them, set the options in your `.mdl` file. The player count and reach of a low-memory device are `low_ram_slots` and `low_ram_reach`.

```ruby
cache do
  memory 128.mib
end

video :moment_video do
  pool slots: 3, reach: 1, trim_delay: 30.seconds, low_ram_slots: 2, low_ram_reach: 0
end
```

On critical pressure the core also calls your `MemoryReleaser`, so Nuke and Coil can drop their memory caches.

## Looking at memory

`stats()` reports slots per level, estimated memory and the number of releases with their reason (timer, pressure, background, source change).

### Swift

```swift
let stats = try await media.stats()
print(stats.slots.hot, stats.slots.warm, stats.slots.cold, stats.memoryBytes)
```

### Kotlin

```kotlin
val stats = media.stats()
println("${stats.slots.hot} ${stats.slots.warm} ${stats.slots.cold} ${stats.memoryBytes}")
```

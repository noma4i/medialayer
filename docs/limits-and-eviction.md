# Limits and eviction

Every size, window, timeout and concurrency value is a setting with a default. An invalid value fails when the library opens. The URL is the cache key, and the cache lives on disk; image memory caches stay with Nuke and Coil.

| Setting | Meaning |
| ------- | ------- |
| `budget` | Disk size at which eviction starts |
| `min` | Smallest allowed budget |
| `shrink_to` | Share of the budget the cache shrinks to, default 90% |
| `evict` | Strategy: `lru`, `fifo`, `lfu`, `largest_first`, `expire_after` |

| Strategy | Removes first |
| -------- | ------------- |
| `lru` | The entry used longest ago |
| `fifo` | The entry stored first |
| `lfu` | The entry used the fewest times |
| `largest_first` | The biggest entry |
| `expire_after` | Entries older than the given age, then the oldest |

The strategy is set once for the cache and can be overridden per media kind (`image`, `video`, `audio`). The same options, with defaults, are written in the `.mdl` file: budget 4 GiB, min 64 MiB, shrink_to 90%, strategy `lru`, memory 256 MiB, timeout 30 seconds, 2 concurrent prefetch loads, repeat skip window 60 seconds and 500 entries. Entries being read or written are never evicted. If new data still does not fit, the call fails with `NotEnoughSpace`.

## Controls

| Call | Result |
| ---- | ------ |
| `limits` | Current limits |
| `setLimits(...)` | Applies and saves new limits at once, without a restart: a smaller budget evicts, the timeout and the number of parallel loads change in the proxy, the number of player slots changes in the pool |
| `resetLimits()` | Back to the defaults from your media file |
| `stats()` | Bytes and files, in total and per media kind |
| `clear(...)` | Removes everything or one media kind. Clearing everything or the images also empties the memory of the image decoder |
| `purge(url)` | Removes one source |
| `evictNow()` | Runs eviction now |

### Swift

```swift
var limits = await media.limits()
limits.budget = 2 * 1024 * 1024 * 1024
limits.evict = .lfu
try await media.setLimits(limits: limits)

let stats = await media.stats()
print(stats.totalBytes, stats.fileCount)

await media.clear(kind: .video)
await media.purge(url: url)
```

### Kotlin

```kotlin
val limits = media.limits().copy(budget = 2UL * 1024UL * 1024UL * 1024UL, evict = Evict.Lfu)
media.setLimits(limits)

val stats = media.stats()
println("${stats.totalBytes} ${stats.fileCount}")

media.clear(Kind.VIDEO)
media.purge(url)
```

Limits are saved in the platform's local storage (UserDefaults on iOS, SharedPreferences on Android) through the `SettingsStore` you pass to `open`. Defaults come from your media file, and a saved value wins over the default. An invalid saved value is dropped and the default is used.

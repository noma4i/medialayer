# Limits and eviction

Every size, window, timeout and concurrency value is a setting with a default. An invalid value fails when the library opens. The URL is the cache key, and the cache lives on disk; image memory caches stay with Nuke and Coil.

| Setting | Meaning |
| ------- | ------- |
| `budget` | Disk size at which eviction starts |
| `min` | Smallest allowed budget |
| `shrink_to` | Share of the budget the cache shrinks to, default 90% |
| `evict` | Strategy: `lru`, `fifo`, `lfu`, `largest_first`, `expire_after` |

The strategy is set once for the cache and can be overridden per media kind (`image`, `video`, `audio`). Entries being read or written are never evicted. If new data still does not fit, the call fails with `NotEnoughSpace`.

## Controls

| Call | Result |
| ---- | ------ |
| `limits` | Current limits |
| `setLimits(...)` | Applies and saves new limits; a smaller budget evicts at once |
| `resetLimits()` | Back to the defaults from your media file |
| `stats()` | Bytes and files, in total and per media kind |
| `clear(...)` | Removes everything or one media kind |
| `purge(url)` | Removes one source |
| `evictNow()` | Runs eviction now |

### Swift

```swift
var limits = media.limits
limits.budget = .gib(2)
limits.evict = .lfu
try await media.setLimits(limits)

let stats = await media.stats()
print(stats.totalBytes, stats.fileCount)

await media.clear(.video)
await media.purge(url)
```

### Kotlin

```kotlin
val limits = media.limits.copy(budget = 2.gib, evict = Evict.Lfu)
media.setLimits(limits)

val stats = media.stats()
println("${stats.totalBytes} ${stats.fileCount}")

media.clear(Kind.Video)
media.purge(url)
```

Limits are saved in the platform's local storage (UserDefaults on iOS, SharedPreferences on Android) through the `SettingsStore` you pass to `open`. Defaults come from your media file, and a saved value wins over the default. An invalid saved value is dropped and the default is used.

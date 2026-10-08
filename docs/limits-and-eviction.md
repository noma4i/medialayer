# Limits and eviction

Every size, window, timeout and concurrency value is a setting with a default. An invalid value fails when the library opens.

| Setting | Meaning |
| ------- | ------- |
| `budget` | Disk size at which eviction starts |
| `min` | Smallest allowed budget |
| `shrink_to` | Share of the budget the cache shrinks to, default 90% |
| `evict` | Strategy: `lru`, `fifo`, `lfu`, `largest_first`, `expire_after` |

The strategy is set once for the cache and can be overridden per media kind (`image`, `stream`, `audio`). Entries being read or written are never evicted. If the new data still does not fit, the call fails with `NotEnoughSpace`.

## Controls

| Call | Result |
| ---- | ------ |
| `limits()` | Current limits |
| `setLimits(...)` | Applies and saves new limits; a smaller budget evicts at once |
| `resetLimits()` | Back to the defaults from your media file |
| `stats()` | Bytes and files, in total and per media kind |
| `clear(...)` | Removes everything or one media kind |
| `purge(url)` | Removes one source |

Limits are saved in the platform's local storage (UserDefaults on iOS, SharedPreferences on Android) through the `SettingsStore` interface. Defaults come from your media file. A saved value wins over the default.

## Keys

The URL is the cache key. The cache lives on disk; image memory caches stay with Nuke and Coil. Headers join the key only if you ask for it with `namespace headers`.

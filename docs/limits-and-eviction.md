# Limits and eviction

Every size, window, timeout and concurrency value is a setting with a default. An invalid value fails when the library opens.

| Setting | Meaning |
| ------- | ------- |
| `budget` | Disk size at which eviction starts |
| `min` | Smallest allowed budget |
| `shrink_to` | Share of the budget the cache shrinks to, default 90% |
| `evict` | Strategy: `lru`, `fifo`, `lfu`, `largest_first`, `expire_after` |

The strategy is set once for the cache and can be overridden per media kind (`image`, `stream`, `audio`). Entries being read or written are never evicted. If the new data still does not fit, the call fails with `NotEnoughSpace`.

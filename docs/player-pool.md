# Player pool

MediaLayer gives you a pool of native players and juggles them for you. You say which pages are visible and which one plays. The pool decides which player holds which source, reuses players when the window moves and never creates more than the pool size.

You normally do not call pool operations. You declare places with `MediaVideo` ([declaring media](declaring-media.md)) and the pool runs them from visibility. Two calls are public.

| Call | Result |
| ---- | ------ |
| `pin(key)`, `unpin(key)` | Keeps a slot for a key, for example an intro video |
| `releaseAll()` | Frees every slot, for example on logout |

### Swift

```swift
media.pin(key: introUrl)
media.unpin(key: introUrl)
media.releaseAll()
```

### Kotlin

```kotlin
media.pin(introUrl)
media.unpin(introUrl)
media.releaseAll()
```

Policies decide assignment: `ring` keeps a window around the active page, `lru` keeps the most recently used keys, `pinned` keeps chosen keys. Retention levels: `hot` plays or is next, `warm` keeps metadata and a small buffer, `cold` holds nothing. The pool size is a setting, and a low-memory device gets fewer slots. Your factory creates each native player once and the pool reuses it for its whole life.

| Setting | Meaning |
| ------- | ------- |
| `slots` | Number of players in the pool, default 3 |
| `low_ram_slots` | Number of players on a low-memory device, default 2 |
| `low_ram_reach` | Reach on a low-memory device, default 0 |
| `reach` | Pages kept ready on each side of the active page, default 1 |
| `hold_off` | Time before a slot released under memory pressure loads again, default 1 second |
| `trim_delay` | Time before an idle slot drops from hot to warm to cold, default 30 seconds |

Slot and reach changes made with `setLimits` apply at once: extra players are unloaded and the window is rebuilt at the next update. These are written in the `pool` line of the `video` block of your `.mdl` file.

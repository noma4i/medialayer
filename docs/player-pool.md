# Player pool

MediaLayer gives you a pool of native players and juggles them for you. You say which pages are visible and which one plays. The pool decides which player holds which source, reuses players when the window moves and never creates more than the pool size.

You normally do not call pool operations. You declare places with `MediaVideo` ([declaring media](declaring-media.md)) and the pool runs them from visibility. Two calls are public.

| Call | Result |
| ---- | ------ |
| `pin(key)`, `unpin(key)` | Keeps a slot for a key, for example an intro video |
| `releaseAll()` | Frees every slot, for example on logout |

### Swift

```swift
media.pool.pin(IntroVideo.key)
media.pool.releaseAll()
```

### Kotlin

```kotlin
media.pool.pin(IntroVideo.key)
media.pool.releaseAll()
```

Policies decide assignment: `ring` keeps a window around the active page, `lru` keeps recent keys, `pinned` keeps chosen keys. The pool size is a setting, and a low-memory device gets fewer slots. Your factory creates each native player once and the pool reuses it for its whole life.

| Setting | Meaning |
| ------- | ------- |
| `slots` | Number of players in the pool |
| `reach` | Pages kept ready on each side of the active page |
| `hold_off` | Time before a released slot loads again |
| `trim_delay` | Time before an idle slot drops from hot to warm to cold |

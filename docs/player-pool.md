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

## Cost of a window

The pool keeps the list you declared with `replaceWindow`. `moveWindow` takes only the active page, so a scroll step costs the size of the window, not the length of your list. `replaceWindow` takes the list and the active page in one call: call it when the sources change. The pool copies only the pages it needs, so a feed of 2000 pages costs the same as a feed of 20 when the list does not change. Image windows work the same way with `replaceImageWindow` and `moveImageWindow`. A move before any list was declared, or after `releaseAll`, does nothing.

## Preferences of a place

The mute and the rate of a source live while a place declares the source or a player holds it. When the place leaves and the player unloads, the pool forgets them, so a long session does not grow. A place that comes back sends its mute and rate again.

## Window is protected

The pages of the window around the active page always keep their players. A place outside the window that becomes visible takes a free player only. It never takes one from the window. A visible place inside the window joins the window.

## Position of a place that left

A place that leaves the screen while it is not playing keeps its position. When the same source comes back, the new player starts at that position. A playing place keeps its position too when the whole window is cleared (the screen is covered by another screen) and restarts from the beginning when it only scrolls out of the window. The library keeps the last 32 positions.

# Player pool

MediaLayer gives you a pool of native players and juggles them for you. You say which pages are visible and which one plays. The pool decides which player holds which source, reuses players when the window moves and never creates more than the pool size.

You normally do not call pool operations. You declare places with `MediaVideo` ([declaring media](declaring-media.md)) and the pool runs them from visibility. Two things are public.

| Call | Result |
| ---- | ------ |
| `pinned` in the input of a place | Keeps a slot for the key of the place while the place lives, for example an intro video |
| `releaseAll()` | Frees every slot, for example on logout |

### Swift

```swift
let intro = try media.openPlace(item: introItem)
try intro.update(input: PlaceInput(visible: false, playing: false, group: nil, peeking: false, muted: nil, rate: nil, pinned: true, viewToken: nil))
intro.release()
media.releaseAll()
```

### Kotlin

```kotlin
val intro = media.openPlace(introItem)
intro.update(PlaceInput(false, false, null, false, null, null, true, null))
intro.release()
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

Slot and reach changes made with `setLimits` apply at once: extra players are unloaded and destroyed, and the window is rebuilt at the next update. These are written in the `pool` line of the `video` block of your `.mdl` file.

## Cost of a window

The pool keeps the list a window object declared. Declare the list with `.mediaWindow(of:rows:source:active:enabled:)` in SwiftUI (an observable model and a key path to its rows; another model mounts a new window) or `MediaWindow(data, source, active, enabled)` in Compose: the library reads your rows, maps them to items off the main thread with a pure function of a row (the latest function serves the next send), and sends only the active index when `active` changes. A feed of 2000 pages costs the same as a feed of 20 while the list does not change. Image windows work the same way with `.mediaImageWindow(name:of:rows:sources:rowsPerPage:reversed:active:enabled:ready:)` and `MediaImageWindow`; without `active` they follow the visible rows of the list (see [media files](media-files.md)); the video window and the image window report `ready` once the active page is prepared; the name picks the prefetch policy, and two windows with the same name stay independent. Several video windows can be mounted: the last window that declared and is `enabled` drives the ring, and the previous live window returns when it closes or is disabled. At the Rust level a `MediaWindow` object takes `replace`, `replace_pages`, `replace_rows`, `show_rows`, `move_to`, `set_active` and `subscribe_ready`.

## Preferences of a place

The mute and the rate of a source live while a place declares the source or a player holds it. When the place leaves and the player unloads, the pool forgets them, so a long session does not grow. A place that comes back sends its mute and rate again.

## Window is protected

The pages of the window around the active page always keep their players. A place outside the window that becomes visible takes a free player only. It never takes one from the window. A visible place inside the window joins the window.

## Position of a place that left

A place that leaves the screen while it is not playing keeps its position. When the same source comes back, the new player starts at that position. A playing place keeps its position too when the whole window is cleared (the screen is covered by another screen) and restarts from the beginning when it only scrolls out of the window. The library keeps the last 32 positions.

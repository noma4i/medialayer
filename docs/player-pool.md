# Player pool

MediaLayer gives you a pool of native players and juggles them for you. You say which pages are visible and which one plays. The pool decides which player holds which source, reuses players when the window moves and never creates more than the pool size.

| Operation | Result |
| --------- | ------ |
| `window(active, keys)` | Recomputes assignments; the pool loads, prerolls, reuses and unloads slots |
| `play(key)` | One slot plays, the others pause; the handoff leaves no empty frame |
| `handoff(from, to)` | Passes playback to a neighbor while scrolling |
| `pin(key)`, `unpin(key)` | Keeps a slot for a key, for example an intro video |
| `release(key)`, `releaseAll()` | Frees one slot or all of them |
| `attach(key, view)` | Binds a native view to a slot without reloading the source |

Policies decide assignment: `ring` keeps a window around the active page, `lru` keeps recent keys, `pinned` keeps chosen keys. The pool size is a setting, and a low-memory device gets fewer slots. Players are created by your factory once and reused for the lifetime of the pool.

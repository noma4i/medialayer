# Video output measurement

Pass `videoOutputCounters = VideoOutputCounters()` to `MediaLayer.open` and retain that instance for the measurement.
Read `snapshot()` on the main thread because it inspects attached Android Views.
Counters change on slot operations and View bindings, without polling or per-frame callbacks.
Without counters `MediaVideo` still wraps the player in `VideoOutputPlayer`, which only forwards.

| Counter | Meaning |
| --- | --- |
| `create`, `release`, `livePlayers` | ExoPlayer creations, releases and their difference |
| `prepare` | Actual prepare calls, including preloading |
| `attach`, `detach`, `repeatAttach` | Slot commands and identical consecutive attachments |
| `setVideoTextureView`, `clearVideoTextureView` | Media3 TextureView binding calls |
| `textureCreate` | Distinct Java Views observed during binding |
| `liveTextures` | Observed Views currently attached to a window |
| `firstFrame` | Rendered-first-frame callbacks |

Missing counters mean zero events. Use a fresh counters instance for each opening.
Distinct Views do not measure graphic buffers or prove garbage collection of detached objects.
Texture removes its View when its player becomes null. Separate pager compositions have separate Views.
Changing mute does not rebind a stable player.

`stats().slots.playing` counts playing or peeking slots on both platforms, not audible players or decoded frames.
Hot slots include retained neighbour buffers. Ordinary neighbours receive Preroll rather than Play.
Explicit peeking plays the neighbour muted. Restored trimmed slots receive Retain.

## Trace reading

Include the application in Perfetto `atrace_apps`.
Read `MediaLayer.create`, `MediaLayer.prepare`, `MediaLayer.attach`, `MediaLayer.detach` and `MediaLayer.firstFrame`.
Measured Texture binding emits `MediaLayer.setVideoTextureView` and `MediaLayer.clearVideoTextureView`.
Run `trace_processor_shell trace.perfetto-trace -q counts.sql`.
Use `SELECT name, COUNT(*) FROM slice WHERE name GLOB 'MediaLayer.*' GROUP BY name` in `counts.sql`.
Restrict timestamps to the marked interval before comparing trace counts with counter differences.

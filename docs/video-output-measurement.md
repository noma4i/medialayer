# Video output measurement

Pass `videoOutputCounters = VideoOutputCounters()` to `MediaLayer.open` and retain that instance for the measurement.
Read `snapshot()` on the main thread because it inspects attached Android Views.
Counters change on slot operations and View bindings, without polling or per-frame callbacks.
Default Texture without counters does not allocate a diagnostic `ForwardingPlayer`.
Window uses `VideoOutputPlayer` to configure the public outline and attachment lifetime, even without counters.

| Counter | Meaning |
| --- | --- |
| `create`, `release`, `livePlayers` | ExoPlayer creations, releases and their difference |
| `prepare` | Actual prepare calls, including preloading |
| `attach`, `detach`, `repeatAttach` | Slot commands and identical consecutive attachments |
| `setVideoTextureView`, `clearVideoTextureView` | Media3 TextureView binding calls |
| `setVideoSurfaceView`, `clearVideoSurfaceView` | Media3 SurfaceView binding calls |
| `textureCreate`, `surfaceCreate` | Distinct Java Views observed during binding |
| `liveTextures`, `liveSurfaces` | Observed Views currently attached to a window |
| `firstFrame` | Rendered-first-frame callbacks, including output changes |

Missing counters mean zero events. Use a fresh counters instance for each opening.
Distinct Views do not measure graphic buffers or prove garbage collection of detached objects.
One stable composition retains its View through a null player. Separate pager compositions have separate Views.
Changing output type can create another View. Changing mute does not rebind a stable player.

`stats().slots.playing` counts playing or peeking slots on both platforms, not audible players or decoded frames.
Hot slots include retained neighbour buffers. Ordinary neighbours receive Preroll rather than Play.
Explicit peeking plays the neighbour muted. Restored trimmed slots receive Retain.

## Enable the experiment

The only opt-in is `output = MediaOutput.Window` on `MediaVideo`. The default remains `MediaOutput.Texture`.
Use `MediaVideo(source, output = MediaOutput.Window, cornerRadius = 16.dp, alpha = 1f)` for the rounded experiment.
Window is selected only on API 34+ with explicit `alpha == 1f`.
API 30/31 and every API below 34 select Texture. Non-opaque alpha selects Texture on every API.
The parameter `cornerRadius` applies the same requested radius to Compose and the public native outline.
Rounding is experimental: visible corners are not proven without a device run.
Parent modifiers are not inspected. Declare radius and opacity explicitly, or retain Texture.
Use Texture throughout continuous cross-fades, including opaque endpoints, to avoid switching View type during the transition.
No shell flag enables this library parameter. Change the call site and rebuild the application with its existing build command.
Swift uses AVPlayerLayer and has no TextureView equivalent or corresponding output flag.

## Samsung SM-A166P procedure

Run on Android 16 with the same build mode, media IDs, cached data, scroll steps, refresh rate and thermal starting condition.
Complete 5 ABBA blocks: A is Texture, B is Window, giving 20 traces with 10 samples per choice.
Run no concurrent profiling or build work. Report every sample, median, minimum and maximum for each choice.
Capture 30 pager swipes, reverse navigation and a stationary video interval in each sample.
Wait for the first frame before the measured interval. Mark interval boundaries in Perfetto.
Use Frame Timeline missed application deadlines at the actual display period, not a fixed 16.7 ms threshold.
The target is at most 5% missed frames with no visual, audio or stability regression.
Android and Media3 do not supply an estimate of frames saved by removing TextureView copying.
The observed 26.07% BlitStreamAUX RenderThread self-time cannot be converted into a frame-saving percentage.

1. Record counters and slot statistics before and after each interval, outside frame measurements.
2. Compare BlitStreamAUX calls and self-time, RenderThread work, GPU work and application-deadline misses.
3. Compare prepare, attach, binding and creation counts, first-frame events and maximum playing slots.
4. Toggle mute while stationary and check that binding, prepare and attach counts remain unchanged.
5. Release the page, then the library. Check attached View counts and `livePlayers = 0` after main-queue processing.
6. Record corners over a transparent page and a contrasting patterned background, without adding a colour mask.
7. Place translucent controls, another moving page and a dialog over the video. Check corners and Z-order frame by frame.
8. Exercise slow drags, rapid flings, reversals, resizing, background/foreground and reopening the page.
9. Exercise alpha 1 to 0 to 1 and overlapping cross-fades. Check first frame, black rectangles, flashes and sound continuity.
10. Repeat API 30/31 checks: Window requests must bind TextureView and emit `MediaLayer.output.Texture.api`.

Reject Window for the tested place if corners stay square, fling flickers, positionChanged crashes, or fade produces a black rectangle.
Also reject missing or stale first frames, double audio, incorrect overlays, increased binding churn or missed frames above the target.
Keep Texture after rejection. Do not change the pool or add an application-owned player.
JVM tests prove selection, configuration and binding behavior, not visible pixels or device stability.

## Trace reading

Include the application in Perfetto `atrace_apps`.
Read `MediaLayer.create`, `MediaLayer.prepare`, `MediaLayer.attach`, `MediaLayer.detach` and `MediaLayer.firstFrame`.
Window binding emits `MediaLayer.setVideoSurfaceView` and `MediaLayer.clearVideoSurfaceView` even without counters.
Measured Texture binding emits `MediaLayer.setVideoTextureView` and `MediaLayer.clearVideoTextureView`.
Selection emits `MediaLayer.output.Window`, `MediaLayer.output.Texture.api` or `MediaLayer.output.Texture.alpha` when the explicit choice changes.
Default Texture does not emit a selection event.
Run `trace_processor_shell trace.perfetto-trace -q counts.sql`.
Use `SELECT name, COUNT(*) FROM slice WHERE name GLOB 'MediaLayer.*' GROUP BY name` in `counts.sql`.
Restrict timestamps to the marked interval before comparing trace counts with counter differences.

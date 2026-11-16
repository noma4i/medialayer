# Testing your app

The library gives app tests a fake fetcher and a player snapshot. Add both only to the app test target.

## Setup

| Platform | Where to add | What it gives |
| --------- | -------------- | -------- |
| Swift | Product `MediaLayerTesting` in the test target | `TestFetcher` |
| Kotlin | `testImplementation(project(":testing"))` or `androidTestImplementation("com.noma4i:medialayer-testing:X.Y.Z")` | `TestFetcher` (`com.noma4i.medialayer.testing`) |
| Swift | `MediaLayerUI` | `LoadedSlots.snapshot(for:)`, `PlayerSnapshot` |
| Kotlin | `kotlin/ui` | `MediaLayer.slotPlayers?.snapshot(key)`, `PlayerSnapshot` |

## TestFetcher

`TestFetcher` returns the given bytes, file or asset by URL and answers `Range` correctly.

| Request | Response |
| ------ | ----- |
| No `Range` | 200, `Content-Length`, `Accept-Ranges: bytes` |
| `Range` inside the body | 206, `Content-Range: bytes s-e/total`, inclusive end |
| End past the body | 206, end clamped to the last byte |
| Start past the body | 416, `Content-Range: bytes */total`, empty body |
| URL not set | error `Fetch("offline")` |

| Swift | Kotlin | Meaning |
| ----- | ------ | ----- |
| `serve(_:at:contentType:)` | `serve(url, data, contentType)` | bytes |
| `serveFile(_:at:contentType:)` | `serveFile(url, file, contentType)`, `serveAsset(url, assets, name, contentType)` | file, app asset |
| `delay(_:for:)` | `delay(url, millis)` | delay before the response, cancel interrupts it |
| `fail(_:detail:)` | `fail(url, detail)` | `Fetch` error for the URL |
| `requests`, `requestCount(for:)` | `requests`, `requestCount(url)` | all requests in arrival order |

## Player snapshot

`PlayerSnapshot(key, isPlaying, isMuted, positionMs, rate)` reads the state of the slot that holds the key. `isPlaying` means a play command without a pause, not that frames are drawn. No slot for the key: `nil`. Reading does not need the main thread.

Swift: `LoadedSlots.of(places).snapshot(for: key)`. Kotlin: `media.slotPlayers?.snapshot(key)`.

## Trace

The core and the player slots can print the order of their events. Android: `adb shell setprop log.tag.MediaLayerTrace VERBOSE`, start the app, then read `adb logcat -s MediaLayerTrace`. The library reads the property when it opens. iOS: start the app with `SIMCTL_CHILD_ML_TRACE=1`. Each line is `ML <ms> <source> <text>`: proxy requests, slot commands, player events and why an `ended` event was dropped. With the property unset the trace does no work.

To see why a video page flickers when the page changes, read the `view` lines of that page in order: `view onScreen <key tail> true|false` (visibility reached the view), `view state <key tail> <state>` (the state the view drew, `loading`, `ready`, `rendered`) and `view placeholder <key tail> cover|blur|none` (the picture under the video appeared or went away). A seamless page shows `onScreen true`, then `state rendered` with `placeholder none` or no placeholder line at all. A page that flickers shows `placeholder cover` after `state ready`, or `onScreen false` and `true` again between two page changes (a parent that hides or moves the page).

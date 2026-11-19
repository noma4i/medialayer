# Testing your app

The library gives app tests a fake fetcher and a player snapshot. Add both only to the app test target.

## Setup

| Platform | Where to add | What it gives |
| --------- | -------------- | -------- |
| Swift | Product `MediaLayerTesting` in the test target | `TestFetcher`, `MediaAudioDriver` |
| Kotlin | `testImplementation(project(":testing"))` or `androidTestImplementation("com.noma4i:medialayer-testing:X.Y.Z")` | `TestFetcher`, `MediaAudioDriver` (`com.noma4i.medialayer.testing`) |
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

## MediaAudioDriver

`MediaAudioDriver(media)` feeds a system audio event into the opened MediaLayer through the same inputs the system signals use. `VoiceState`, `CameraState` and `microphoneAvailability` run their real code: no real call, no muted microphone, no revoked permission needed on the device. The entry is closed to the app code: Swift marks it `package` (only `MediaLayerTesting` sees it), Kotlin marks it with an opt-in that is an error without `@OptIn(MediaLayerTestingApi::class)` and only `:testing` uses it. Use the driver in tests, not in app code.

| Event | Swift | Kotlin | A live take ends with | `microphoneAvailability` |
| ----- | ----- | ------ | ---------------- | ---------- |
| A call of another app | `startCall()`, `endCall()` | same | `inCall` | `externalCall`, then `available` |
| The microphone gives no sound | `silenceMicrophone()`, `restoreMicrophone()` | same | `microphoneSilent` | `noSound` while an input is open and silent, then `available` |
| The system interrupts the audio | `beginInterruption()`, `endInterruption(shouldResume:)` | `beginInterruption()`, `endInterruption()` | `interrupted` | not `available`, then `available` |
| The permission is taken away | `revokePermission()`, `grantPermission()` | same | `permissionDenied` | `permissionDenied`, then `available` |

`revokePermission` and `grantPermission` change only the state of the library, not the system permission: the next real read of the system permission (foreground, the app asks again) puts the real value back. `endInterruption(shouldResume:)` carries the iOS resume hint; Android has no hint and the audio focus simply returns.

Recipe: open MediaLayer as the app does, start the take (`MediaVoiceRecorder.start`, `MediaCamera.startRecording`), call the driver, then read the state. A camera or voice take reports its refusal in `VoiceState.outcome` / `CameraState.outcome`, a start refused during the event throws the same refusal.

```swift
let recorder = MediaVoiceRecorder(maxDuration: 120)
let driver = MediaAudioDriver(media)
try await recorder.start(to: fileURL)
driver.startCall()
// recorder.state.outcome == .refused(reason: .inCall)
// media.microphoneAvailability yields .externalCall
driver.endCall()
```

```kotlin
val recorder = MediaVoiceRecorder(context, maxDurationMs = 120_000)
val driver = MediaAudioDriver(media)
recorder.start(file)
driver.silenceMicrophone()
// recorder.state.value.outcome == VoiceOutcome.Refused(VoiceRefusal.MICROPHONE_SILENT)
driver.restoreMicrophone()
```

## Player snapshot

`PlayerSnapshot(key, isPlaying, isMuted, positionMs, rate)` reads the state of the slot that holds the key. `isPlaying` means a play command without a pause, not that frames are drawn. No slot for the key: `nil`. Reading does not need the main thread.

Swift: `LoadedSlots.of(places).snapshot(for: key)`. Kotlin: `media.slotPlayers?.snapshot(key)`.

## Trace

The core and the player slots can print the order of their events. Android: `adb shell setprop log.tag.MediaLayerTrace VERBOSE`, start the app, then read `adb logcat -s MediaLayerTrace`. The library reads the property when it opens. iOS (Debug builds only; a release build or TestFlight app must not set the key, so it prints nothing): start the app with `SIMCTL_CHILD_ML_TRACE=1` (simulator), or set the `MLTrace` user default, the same as the launch argument `-MLTrace YES`; the app can set it in a Debug or TestFlight build. The library reads it once, at the first line. Lines go to the unified log with subsystem `com.noma4i.medialayer` and the source as category (`core` for the core): read them in Console.app (pick the iPhone), or for a simulator `log stream --predicate 'subsystem == "com.noma4i.medialayer"'`. A line keeps only the last 24 characters of a url path, never the query, a header or an error text. Each line is `ML <ms> <source> <text>`: proxy requests, slot commands, player events and why an `ended` event was dropped. With the property unset the trace does no work.

To see why a video page flickers when the page changes, read the `view` lines of that page in order: `view onScreen <key tail> true|false` (visibility reached the view), `view state <key tail> <state>` (the state the view drew, `loading`, `ready`, `rendered`) and `view placeholder <key tail> cover|blur|none` (the picture under the video appeared or went away). A seamless page shows `onScreen true`, then `state rendered` with `placeholder none` or no placeholder line at all. A page that flickers shows `placeholder cover` after `state ready`, or `onScreen false` and `true` again between two page changes (a parent that hides or moves the page).

# App migration

What an app changes when it moves to a new MediaLayer version. Each section names the removed API, its replacement, and what the app shows.

## Next version

- Unreleased, iOS: a video take is recorded by the system writer (`AVCaptureMovieFileOutput`) again, as in 3.33.1. The microphone is no longer watched while the take runs, so iOS no longer ends a take with `RecordEnd.microphoneLost` and no longer refuses a clip for a hole in its sound; a take whose microphone fails ends as `recordFailed`. Nothing else changes in the API.
- `microphoneAvailability` no longer blinks `noSound` when a microphone starts: `noSound` comes only from a silence the platform reports. An app that disables recording on `noSound` no longer cancels its own start.
- `microphoneBusy` comes only from a call, from a voice start during a camera take (and a camera take during a voice), and from a second start of the same recorder. A recording of another app shows `otherApp` and no longer refuses a start.
- A voice start right after a camera take, while the preview lives, records: the voice takes the microphone of the idle preview, and the preview gets it back after the voice.
- 3.43.0: a recorded video stands as its preview showed it, whatever the pose of the phone. The app rotates nothing and needs no change.
- 3.43.0, iOS: trace lines go to the unified log (subsystem `com.noma4i.medialayer`) when the app sets the `MLTrace` user default before its first MediaLayer call. Set it in Debug builds only: without the key the trace does nothing.
- 3.44.0: the ring of a recorder is drawn by the library. Swift `MediaTakeProgress(camera)` and `MediaVideoProgress(source)`, Kotlin `rememberTakeProgress(camera)` and `rememberPlayheadProgress(source)` give a fraction from 0 to 1 at the display frame rate. Remove a ring driven by `recordedMs` or `onProgress`, and any app timer for it.
- 3.45.0: the review of a camera clip opens right after stop: the sound is judged while the take records and iOS writes upright frames, so the file is not read back or re-encoded. Nothing changes for the app. `CameraFrame.orientation` of the iOS frame tap is now `.up` (`.upMirrored` for an unmirrored front lens): an app that reads tap frames must use the field and not assume `.right`.
- 3.45.0: the voice recorder has the same smooth ring: Swift `MediaTakeProgress(recorder)`, Kotlin `rememberTakeProgress(recorder)`.
- 3.46.0: a refused memory reservation of the camera or voice tells its cause. A closed MediaLayer refuses with `notReady` (was `memory`); `memory` now means the budget is full; any other failure is `notReady` with a native failure in `Media.failures`. An app that shows "not enough memory" on `memory` now shows it only when it is true.
- 3.46.0: a camera clip keeps its whole sound on iOS: the stop waits until the sound reaches the last frame. The front camera of iPhone 17 records upright. Nothing changes for the app.
- 3.47.0: a take whose microphone is lost mid-take (headphones taking the route, a system stop, a flow loss after the start) ends by itself when the video runs 1 s ahead of the written sound; the clip arrives the usual way (`stopRecording`, outcome captured) cut at the end of its sound. `CapturedClip.end` tells how the take ended: Swift `.completed`, `.limitReached`, `.microphoneLost`; Kotlin `RecordEnd.COMPLETED`, `LIMIT_REACHED`, `MICROPHONE_LOST`. A take shorter than `profile.minTakeMs` is still refused `tooShort`; a take that never had sound is still refused `microphoneSilent`. Show the clip and, on `microphoneLost`, a note that the recording stopped because the microphone was lost.
- 3.47.0: the camera preview is transparent until its first frame: what the app draws under `MediaCameraPreview` stays visible during the open.

## 3.41.0

An app that moves from 3.33.1 or earlier makes every change below at once. MediaLayer owns the audio session (iOS) and the audio focus (Android) for playback, recording and calls, and every change it makes by itself reaches the app as a stream or a state on both platforms. The app keeps no audio session, focus, call, microphone or playing state of its own, no take timer, and polls nothing.

### Remove

| Removed | Replacement |
| ------- | ----------- |
| `.id(playerRound)` or `key(generation)` to remount a failed place | `Media.retry(source)` |
| `MediaLayer.callHold()`, `MediaCallHold` | Nothing for a phone call or a call of another app: MediaLayer sees it. Your own call: `callAudio` |
| `callAudio(managedBySystem:callUUID:)`, `liveChanges`, Kotlin `callAudio(managedBySystem)`, `live` | `callAudio(kind)` and its `states` |
| `MediaAudioSession.activateRecording()`, `deactivateRecording()`, `holdRecording()`, `MediaRecordingHold` | Nothing: `MediaVoiceRecorder` and `MediaCamera` hold the microphone themselves |
| Your own `AVAudioSession` category or `setActive` calls, your own `requestAudioFocus` | Nothing |
| Your own mute on a lost headphone | Nothing: MediaLayer pauses the players and keeps `Media.muted` as the user set it |
| Swift `SystemSignalNames(memoryWarning:background:foreground:outputLost:)` | `SystemSignalNames(memoryWarning:background:foreground:)`: MediaLayer watches the audio route itself |
| Kotlin `MediaAudioSession`, `Media.audio`, the `audio` argument and property of `ExoPlayerSlotFactory` | `media.readVolume()` and `media.volumeRaises()` on the open MediaLayer. Swift keeps `Media.audio?.readVolume()` and `volumeRaises()` |
| `playUrl(source)` that returned a `PlaybackRoute` | `playUrl(source)` returns a player of MediaLayer with `play()`, `pause()`, `seek(fraction)`, `release()`, `state` and `states`. An app with its own player reads the route with `readRoute(item)` |
| `MediaError.InCall` thrown by `playUrl` during a call | `playUrl` gives the player, its `play()` sets `state.refused` to `inCall` |
| Kotlin `MediaCallKind.Telecom` object | `MediaCallKind.Telecom(CallEndpointPort.of(callControlScope))` with the `CallControlScope` of `addCall` |
| Your own playing flag beside `audio.isPlaying` | `audio.isPlaying` is the truth of the core: false until the player plays, false after a pause by MediaLayer. Swift `AudioPlayback` gets `play()` and `pause()` beside `toggle()` |
| Your own `keepsScreenAwake` (Swift) or `KeepScreenAwake` (Kotlin) from a playing flag | Remove it: the library keeps the screen on while a place plays with sound, a voice take runs, a camera preview is on screen or a call runs, and a muted place does not hold it |
| Your own "video plays" flag | `MediaVideo(onPlaybackChange:)` (Swift), `MediaVideo(onPlaybackChange = ...)` (Kotlin) with `PlaybackState` |
| A wait on `stop()` or `stopRecording()` to learn how a take ended | `MediaVoiceRecorder.states` with `VoiceState.outcome`, `MediaCamera.states` (Kotlin `state`) with `CameraState.take` and `outcome`, the clip from `clip(take:)` |
| `Media.failure` as the feed of an error tracker | `Media.failures`, one `NativeFailure` per error (see [Failures](failures.md)) |
| Kotlin `microphoneAvailability` after close threw an exception | It gives a flow that ends at once, like Swift |
| Swift test doubles of `MediaPlaces` and `PlaceHandle` | `MediaPlaces` adds `readRoute(item:)` and its `playUrl(source:)` returns a player. `PlaceHandle` adds `subscribePlayback(listener:)` |

### Microphone permission and the start of a voice take

MediaLayer never shows the system prompt for the microphone. Your app asks for the permission before it calls `MediaVoiceRecorder.start`, on iOS and on Android. Without the permission `start` throws `permissionDenied` and nothing is recorded.

`start` returns when the first live PCM of the take has arrived and its file is open. Before that no file exists and `phase` stays `idle`. Show the recording screen from the moment `start` returned, not from the call of `start`: a start that waits for sound can end with `refused(microphoneSilent)`, `refused(inCall)` or a cancel, and then it leaves no file. Kotlin `start` is a `suspend` function. A take that ends by `microphoneSilent` releases its microphone at once, and the next `start` opens a new one.

`MediaCamera` with a sound profile follows the same rule: its take shows `recording` only after the microphone sound flows.

### Calls

Take the call audio when your call starts and end it when the call ends. While any call runs, every player is paused and recording is refused. The end of a call restores nothing.

Swift `callAudio(_:)` is `async throws` and throws `MediaError.Closed` after close. Kotlin throws `MediaException.Closed` after close. Closing MediaLayer is not a failure. Ending your own call clears its communication mode. A later communication mode belongs to another app.

```swift
let call = try await media.callAudio(.callKit(callUUID)) // or .unmanaged
call.systemActivated()   // CallKit didActivate
call.systemDeactivated() // CallKit didDeactivate
call.end()
```

```kotlin
val call = media.callAudio(MediaCallKind.Telecom(CallEndpointPort.of(callControlScope))) // or MediaCallKind.Unmanaged
call.systemActivated()   // Telecom onSetActive
call.systemDeactivated() // Telecom onSetInactive
call.close()
```

Start the call audio of WebRTC or of your calling library when `states` gives `live`, and stop it on `interrupted` or `ended`. The full order is in [Calls](declaring-media.md#calls).

### Microphone

Observe `microphoneAvailability` and disable the record buttons while the value is not `available`.

Unexpected session, focus and speaker errors reach `Media.failures` with their system domain and code. iOS `cannotInterruptOthers`, `insufficientPriority` and `cannotStartPlaying` and Android focus `FAILED` and `DELAYED` are states, not failures. A system event or a new demand leaves a refused or delayed state. A newly sounding place, recording, call or volume reader is a new demand.

Loss of a private output pauses playback at any time. Loss means the previous route had a private output and the new route has none. A switch from A2DP to HFP is not a loss.

| Value | What the app shows |
| ----- | ------------------ |
| `available` | Record buttons enabled |
| `noSound` | A started microphone has not given sound yet or went silent: a hint, the take refuses by itself with `microphoneSilent` |
| `ownCall` | Record buttons disabled, your call screen is visible |
| `externalCall` | A banner: the microphone is busy with a call |
| `otherApp` | A banner: another app uses the microphone, or the system interrupted the audio, took the focus or refused the session |
| `permissionNotAsked` | Your own explanation, then the system permission request. Only iOS gives it, Android cannot tell it from a refusal |
| `permissionDenied` | The permission was asked and refused: a prompt that opens the settings. Android gives it also for a permission that was never asked |

An app that read one value for "no permission" now handles `permissionNotAsked` apart: it asks, `permissionDenied` sends to the settings. A `when` over `MicrophoneAvailability` in Kotlin and a `switch` in Swift need the two new cases `noSound` and `permissionNotAsked`.

```swift
for await value in media.microphoneAvailability { recordEnabled = value == .available }
```

```kotlin
media.microphoneAvailability.collect { recordEnabled = it == MicrophoneAvailability.AVAILABLE }
```

### Recording refusals

`VoiceRefusal` and `CameraRefusal` have two new cases. A refused or stopped recording leaves no file.

| Refusal | When | What the app shows |
| ------- | ---- | ------------------ |
| `interrupted` | The system interrupted the audio (an alarm, Siri, a call screen) during the recording, or before it started, or MediaLayer closed or the screen left mid-take | "Recording stopped", the user records again |
| `microphoneSilent` | The microphone gave no sound when a take started, or its sound stopped during the take | "No sound from the microphone", the user records again |
| `audioTrackBroken` | New value: the recorded file's sound track has a hole, is cut, empty or unreadable, so the take was thrown away | Show it as a failed recording, "The recording failed", the user records again |
| `inCall` | A call runs | The call banner |
| `microphoneBusy` | The system refused the recording session, another app records, or a second `start` came during a take | The busy banner |

A second `start` during a take is refused as `MicrophoneBusy` and a `stop` without a take as `Empty`, the running take is untouched. A refusal at the end of a take is an outcome in `VoiceState.outcome`, not a `Media.failures` event.

### Camera clips

`CameraState.outcome` `captured(take)` keeps its clip until the app takes it with `clip(take:)` (Swift) or `clip(take)` (Kotlin) or the camera closes. The next take, a lens change and leaving the screen do not delete it. The clip carries the lens of its take, not the lens of the camera now. Kotlin `clip(take)` can be called from a frame analyzer.

A stop that waits when the camera unbinds or MediaLayer closes ends with `refused(interrupted)`, and a clip delivered before the close is still given. Cancelling `startRecording` cancels its own take on both platforms.

### Playback

Recording pauses every player, and the end of a recording plays nothing back. An interruption that the system allows to resume plays the same places again. A lost headphone or Bluetooth output pauses the players. In all other cases a place plays after its owner declares play again.

### What the app shows

| Signal | What the app shows |
| ------ | ------------------ |
| `audio.isPlaying` false, `PlaybackState.reason` set | The Play button. One press plays again |
| `PlaybackState.refused` is `inCall` or `recording` | The Play button stays, a hint that a call or a recording runs |
| `VoiceState.outcome` is `refused(reason)` | "Recording stopped" with the reason, no file |
| `VoiceState.outcome` is `recorded(path, durationMs)` without a `stop()` | The finished voice message at the longest length |
| `CameraState.outcome` is `captured(take)` | The clip from `clip(take:)` |
| `CameraState.outcome` is `refused(reason)` or `lost(reason)` | "Recording stopped" with the reason |
| `CameraState.phase` is `closed` | The camera screen closes |
| `MediaCallAudio.states` is `interrupted` | Call audio stopped until `live` |
| `MediaCallAudio.route` is `speaker`, `receiver` or `failed` | The speaker button in the state the system applied |
| `microphoneAvailability` not `available` | Record buttons disabled with the busy banner |
| A `Media.failures` event | Nothing on screen, the event goes to the error tracker |

### Tests

`MediaAudioDriver` of the testing modules (Swift `MediaLayerTesting`, Kotlin `:testing`) drives a call, a silent microphone, an interruption and a revoked permission through the real audio link, so a test needs no real call on a device. See [Testing your app](testing-your-app.md#mediaaudiodriver).

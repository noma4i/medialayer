# Camera capture

`MediaCamera` takes photos and records video with a preview, on iOS (`AVCaptureSession`) and Android (CameraX). The app declares a profile and a preview, the library owns the session.
The app has no `prepare`, `resume` or `close` step: the camera runs only while its preview is on screen and the app is in front. Leaving the screen, going to the background
or letting the camera go releases the camera, the microphone focus and the memory budget by itself. A camera at rest holds no session, no timer and no frame output.

The face search (ML Kit, Vision), the mirror of a photo, the permissions and the screen are the app's. The library reads the permission status and answers a refusal with `MediaError.Camera`.

## Profiles

| Profile | Video | Frame rate | Limit | Least take | Sound | Stabilization |
| --- | --- | --- | --- | --- | --- | --- |
| `.photo` | none | 30 | none | none | no | off |
| `.moment` | 1280x720 | 30 | 10 s | 500 ms | yes | off |
| `.circle` | 720x1280 | 30 | 60 s | 0 | yes | cinematic |

A photo is a JPEG of quality 90 (`photoQuality`). `mirrorFrontVideo` mirrors the video of the front camera, `reducesRedEye` asks for red-eye reduction. An invalid profile (a side of zero,
a frame rate outside 1 to 60, a video without a size or with a limit outside 1 s to 10 min, a photo with a limit, sound or stabilization) is refused by the initializer with `MediaError.InvalidSpec`.

`stabilization` is `off`, `standard` or `cinematic`. The profile asks, the device answers: a mode the device does not offer is not an error, the video is recorded with the stabilization the platform keeps
(iOS: the modes of the active format, Android: CameraX video stabilization when the device supports it). A profile is a value, so a different least take is a copy with one field changed
(`var profile = CaptureProfile.circle; profile.minTakeMs = 500` in Swift, `captureProfile(CaptureKind.CIRCLE).copy(minTakeMs = 500UL)` in Kotlin).

## Swift

```swift
struct MomentCamera: View {
  @State private var camera = try? MediaCamera(profile: .moment)

  var body: some View {
    if let camera {
      MediaCameraPreview(camera: camera)
        .task {
          try? await camera.startRecording()
        }
    }
  }
}
```

`camera.state` and `camera.states` give the phase (`idle`, `opening`, `ready`, `recording`, `stopping`, `failed`, `closed`), the length of the recording, flash, zoom, the side, the last refusal, the number of the take and its outcome.
`WithMediaCameraState(camera:)` follows it in a view. `takePhoto()` gives `CapturedPhoto`, `stopRecording()` gives `CapturedClip`, `cancelRecording()`, `setLens(_:)`, `setFlash(_:)` and `zoom(by:)` complete the commands.
A command that does not fit the phase is ignored and `state` shows it.

## Kotlin

```kotlin
@Composable
fun MomentCamera() {
  val camera = rememberMediaCamera(captureProfile(CaptureKind.MOMENT))
  MediaCameraPreview(camera, Modifier.fillMaxSize())
  LaunchedEffect(camera) { camera.startRecording() }
}
```

`camera.state` is a `StateFlow<CameraState>`. `takePhoto()`, `startRecording()` and `stopRecording()` are `suspend`. `rememberMediaCamera` closes the camera when it leaves the composition.

## What a command gives

- `takePhoto` gives an upright JPEG file (the EXIF turn is applied, the mirror is not), its size after the turn and the side it was taken from. Mirror a front photo with `prepareImage(mirror:)`.
- `startRecording` returns when the take is recording: for a profile with sound only after the first live sound of the microphone, at the same moment `CameraState` turns `recording` (Swift and Kotlin). Show the recording UI from either; without sound the call ends with the refusal, never a silent take. The microphone of a take is retried softly like a voice start: up to 3 attempts, each waits at most 1 s for its first sound, 200 ms apart; only the third attempt without sound ends the call with `microphoneSilent`, and the attempts are not published in `CameraState` or `microphoneAvailability`. `stopRecording` gives the clip (file, length, frame as the screen shows it). A recording that is too short, failed or thrown away is a `MediaError.Camera` with the reason. A recording that is too short also carries its length in `recordedMs` (milliseconds, `nil` for every other reason), and `CameraState.refusedMs` holds it until the next recording starts.
  The recording ends by itself at the limit, and `stopRecording` then gives that clip.
- A lens change during a recording throws the recording away and binds the other side. Leaving the screen during a recording throws it away and deletes its file.
- Each recording is a take with a number, `CameraState.take`. `CameraState.outcome` tells how the last take ended, also when nobody called `stopRecording`:

  | Outcome | When | What the app does |
  | --- | --- | --- |
  | `captured(take)` | `stopRecording`, or the limit ended the take | `clip(take:)` (Swift) or `clip(take)` (Kotlin) gives the clip. The clip is given once: by `clip` or by `stopRecording`. A clip nobody fetched stays until `clip(take)` takes it or the camera closes: the next take, a lens change and leaving the screen do not delete it |
  | `refused(reason)` | A call, an interruption, a silent microphone, a take whose sound track is broken (`audioTrackBroken`), a take that is too short, an unrecoverable fault, closing MediaLayer | Shows the reason, the file is deleted |
  | `lost(reason)` | A recoverable fault of the camera during the take | The take is gone, the camera opens again by itself |

  A take with sound starts only after the microphone gave sound: `phase` stays out of `recording` until then. A flash or zoom command that the system refused is a `camera` event of `Media.failures`, and `state` keeps the old flash and zoom.
- Closing MediaLayer ends a running take with `refused(interrupted)`, releases the camera and sets `phase` to `closed`. A closed camera takes no command. Swift `states` ends.
- The files of a capture live in `<cache>/capture` as `capture-<id>.jpg` and `capture-<id>.mp4`. A delivered file belongs to the app, files that nobody took are removed when `open` runs.

## Take progress

`CameraState.recordedMs` steps every 100 ms. For a smooth ring use `MediaTakeProgress` (Swift) or `rememberTakeProgress(camera)` (Kotlin): it is the recorded time against the limit of the profile (`maxDurationMs`), 0 to 1, drawn every display frame. It never steps back, a new take starts from 0, and a finished take stops at its last position.

```swift
struct TakeRing: View {
  let camera: MediaCamera

  var body: some View {
    MediaTakeProgress(camera) { fraction in
      Capsule().frame(width: 200 * fraction, height: 4)
    }
  }
}
```

```kotlin
@Composable
fun TakeRing(camera: MediaCamera) {
  val fraction = rememberTakeProgress(camera)
  Box(Modifier.drawBehind { drawRect(Color.Black, size = androidx.compose.ui.geometry.Size(size.width * fraction.value, 4f)) })
}
```

The voice recorder has the same ring: `MediaTakeProgress(recorder)` (Swift) and `rememberTakeProgress(recorder)` (Kotlin) follow `VoiceState.recordedMs` against the limit of the recorder. A recorder without a limit (0) gives 0.

```swift
struct VoiceRing: View {
  let recorder: MediaVoiceRecorder

  var body: some View {
    MediaTakeProgress(recorder) { fraction in
      Capsule().frame(width: 200 * fraction, height: 4)
    }
  }
}
```

```kotlin
@Composable
fun VoiceRing(recorder: MediaVoiceRecorder) {
  val fraction = rememberTakeProgress(recorder)
  Box(Modifier.drawBehind { drawRect(Color.Black, size = androidx.compose.ui.geometry.Size(size.width * fraction.value, 4f)) })
}
```

## Frames for a face search

`observeFrames(maxPerSecond:_:)` (Swift, a `CVPixelBuffer` and its orientation) and `observeFrames(maxPerSecond, analyzer)` (Kotlin, an `ImageAnalysis.Analyzer`, with `camera.sensorToView`) give the frames of the preview
to the app. The frame output exists only while somebody listens. An observer that starts during a take does not bind the recording again: on Android it gets frames after the take ends. The `onLayer` argument of the Swift `MediaCameraPreview` hands over the AVFoundation preview for the conversion of coordinates.

## Memory and threads

The session asks the Ledger for `capture_cost` before the camera binds, a shot asks for `shot_cost` and gives it back after the photo. A refusal is `MediaError.Camera(.memory)` and the camera is `failed`.
The core, the file work and the clip reading run off the main thread. The binding of CameraX is the only Main work, CameraX requires it.

## Voice file length on Android

`MediaVoiceRecorder` files on Android are AAC in `.m4a`. `MediaMuxer` writes no priming edit list, so the encoder delay (two AAC frames, about 46 ms at 44.1 kHz) stays in the file as leading silence, and the container duration is longer than `durationMs` by that delay. No audio is cut or added. iOS writes the priming information, so its files carry no extra silence.

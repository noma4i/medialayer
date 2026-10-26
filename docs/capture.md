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

`camera.state` and `camera.states` give the phase (`idle`, `opening`, `ready`, `recording`, `stopping`, `failed`), the length of the recording, flash, zoom, the side and the last refusal.
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
- `startRecording` returns after the hardware started. `stopRecording` gives the clip (file, length, frame as the screen shows it). A recording that is too short, failed or thrown away is a `MediaError.Camera` with the reason. A recording that is too short also carries its length in `recordedMs` (milliseconds, `nil` for every other reason), and `CameraState.refusedMs` holds it until the next recording starts.
  The recording ends by itself at the limit, and `stopRecording` then gives that clip.
- A lens change during a recording throws the recording away and binds the other side. Leaving the screen during a recording throws it away and deletes its file.
- The files of a capture live in `<cache>/capture` as `capture-<id>.jpg` and `capture-<id>.mp4`. A delivered file belongs to the app, files that nobody took are removed when `open` runs.

## Frames for a face search

`observeFrames(maxPerSecond:_:)` (Swift, a `CVPixelBuffer` and its orientation) and `observeFrames(maxPerSecond, analyzer)` (Kotlin, an `ImageAnalysis.Analyzer`, with `camera.sensorToView`) give the frames of the preview
to the app. The frame output exists only while somebody listens. The `onLayer` argument of the Swift `MediaCameraPreview` hands over the AVFoundation preview for the conversion of coordinates.

## Memory and threads

The session asks the Ledger for `capture_cost` before the camera binds, a shot asks for `shot_cost` and gives it back after the photo. A refusal is `MediaError.Camera(.memory)` and the camera is `failed`.
The core, the file work and the clip reading run off the main thread. The binding of CameraX is the only Main work, CameraX requires it.

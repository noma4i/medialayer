# Choosing a crop

`MediaCropper` shows a photo and lets the user move and scale a crop frame, on iOS (SwiftUI) and Android (Compose). The result is a rectangle in fractions of the photo and a turn, the same values `PhotoEdit` takes as `crop` (a `CropFractions`) and `rotationDegrees`.
The library draws the photo, the dimmed area and the frame, and owns the gestures. The buttons, the captions and the call of `MediaPreparer` are the app's. A crop at rest holds no task, no timer and no animation.

## The two frames

| Frame | Made with | What the user does |
| --- | --- | --- |
| Fixed proportions | `aspect` (1080:1920 for a chat or a moment, 1:1 for an avatar) | The frame stays still and as large as the window allows. Drag and pinch move and scale the photo under it. |
| Free | no `aspect` | Drag a side or a corner to resize the frame, drag inside to move it, pinch to resize it. |

The photo never leaves the frame, the frame never leaves the photo. A fixed frame shrinks to a third of its largest size at most, a free frame is not smaller than 10 percent of the shorter side of the photo (`CropLimits`).

## Swift

```swift
let crop = MediaCrop(aspect: CGSize(width: 1080, height: 1920)) { result in
  guard let result else { return dismiss() }
  let edit = PhotoEdit(maxSize: nil, enlarge: false, crop: CropFractions(x: Double(result.crop.minX), y: Double(result.crop.minY), width: Double(result.crop.width), height: Double(result.crop.height)), outputSize: PixelSize(width: 1080, height: 1920), mirror: false, rotationDegrees: UInt32(result.rotation))
  let descriptor = try? mediaPrepareDescriptor(intent: .image(edit: edit, quality: nil, format: .jpeg))
  Task { if let descriptor { _ = await preparer.run(job: MediaPrepareJob(attemptId: 1, source: path, destination: output, descriptor: descriptor)) { _ in } } }
}

MediaCropper(source: source, crop: crop, shape: .rect)
Button("Turn") { crop.turnLeft() }
Button("Done") { crop.confirm() }
Button("Cancel") { crop.cancel() }
```

## Kotlin

```kotlin
val crop = rememberMediaCrop(aspect = Size(1080, 1920)) { result ->
  if (result == null) dismiss() else scope.launch {
    val frame = result.crop
    val fractions = CropFractions(frame.left.toDouble(), frame.top.toDouble(), frame.width().toDouble(), frame.height().toDouble())
    val edit = PhotoEdit(null, false, fractions, PixelSize(1080u, 1920u), false, result.rotation.toUInt())
    preparer.run(MediaPrepareJob(1u, path, output, mediaPrepareDescriptor(MediaPrepareIntent.Image(edit, null, ImageFormat.JPEG)))) {}
  }
}

MediaCropper(source, crop, shape = MediaCropShape.Rect)
Button(onClick = crop::turnLeft) { Text("Turn") }
Button(onClick = { crop.confirm() }) { Text("Done") }
Button(onClick = crop::cancel) { Text("Cancel") }
```

- `confirm()` gives `MediaCropResult` (`crop` in fractions of the turned photo, `rotation` 0, 90, 180 or 270 clockwise) and calls `onFinish` with it. `cancel()` calls `onFinish` with `nil` (`null`) and returns the transform at that moment as `MediaCropSnapshot`. `onFinish` is called once, a command after that does nothing, and `confirm()` before the photo is shown gives `nil` (`null`).
- `initial` is the first frame (fractions of the photo). It is fitted to the proportions and the limits and keeps its middle. Without it the frame is the largest one in the middle. `reset()` returns to it.
- `turnLeft()` and `turnRight()` turn the photo by a quarter, the frame turns with it. `allowsRotation: false` turns that off. Only turns by 90 degrees exist.
- `shape: .circle` (`MediaCropShape.Circle`) dims the photo outside a circle for an avatar, the result is still a rectangle.
- The source is a local file (`file://`). Anything else, a file that is no image, or a refusal of the memory budget is a `MediaError` in `crop.failure` (`MediaException`). The decode takes its raster from the shared memory budget before it allocates and gives it back when it is done.
- The result passes the core check: the frame is inside the photo, it holds a pixel on both axes and keeps the proportions within one pixel per edge, so the image intent never refuses it.

## Gesture telemetry

The app reports how the user moved the crop (an analytics event, a "was the position changed" flag) without writing its own gesture code.

- `onPositionChange` (a parameter of `MediaCrop` and `rememberMediaCrop`) is called when a gesture ends, once for each kind that took part, with a `CropGestureReport`. A gesture with a pan and a pinch gives two calls in a row, the pan first, then the pinch. A pure pan or a pure pinch gives one call. A touch that moved nothing gives no call. The calls come when the gesture ends, not on every step.
- `kind` is `.pan` (`PAN`) for the report of the move and `.pinch` (`PINCH`) for the report of the scale. A handle drag of a free frame is a pan.
- `durationMs` is the time from the first to the last event of the whole gesture on the monotonic clock of the platform, the same in both reports.
- `maxDistance` is the largest move during the gesture, counted from the frame at its start, as a whole number: view points of the photo (of the frame when it is free) for a pan, the change of the scale for a pinch.
- `scaleChange` is the scale at the end minus the scale at the start of the gesture (rounded to 0.01). `translateChange` is the length of the move between the start and the end of the gesture in whole view points on iOS and pixels on Android.
- `finalScale`, `finalTranslateX` and `finalTranslateY` are the transform against the first frame after the gesture, in the units below.
- `crop.snapshot()` (`MediaCropSnapshot`, `nil` before the photo is shown) gives `crop` and `rotation` (what `confirm()` would give now) and `finalScale`, `finalTranslateX`, `finalTranslateY` and `wasPositionChanged` for the frame now without finishing. The crop stays open: a gesture and `confirm()` work after it, so a screen can keep the area when an upload fails and the user saves again. It is readable after `cancel()`, which returns the same snapshot.
- `MediaCropResult` carries `finalScale`, `finalTranslateX`, `finalTranslateY` and `wasPositionChanged`.
- `finalScale` is 1 for the first frame (the frame of `initial`, or the largest one) and above 1 when the user magnified. A free frame that was resized smaller counts as magnified. It is rounded to 0.01.
- `finalTranslateX` and `finalTranslateY` are the move of the photo under the fixed frame (of the frame itself when it is free) in whole view points on iOS and pixels on Android, measured against the first frame. Right and down are positive.
- `wasPositionChanged` is true once any pan or pinch moved or scaled the crop. Moving back to the first frame does not clear it. A turn and `reset()` do not change it and a turn alone moves nothing.

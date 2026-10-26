# Choosing a crop

`MediaCropper` shows a photo and lets the user move and scale a crop frame, on iOS (SwiftUI) and Android (Compose). The result is a rectangle in fractions of the photo and a turn, the same values `prepareImage` takes as `crop` and `rotation`.
The library draws the photo, the dimmed area and the frame, and owns the gestures. The buttons, the captions and the call of `prepareImage` are the app's. A crop at rest holds no task, no timer and no animation.

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
  Task { photo = try await media.prepareImage(source, crop: result.crop, rotation: result.rotation, outputSize: CGSize(width: 1080, height: 1920)) }
}

MediaCropper(source: source, crop: crop, shape: .rect)
Button("Turn") { crop.turnLeft() }
Button("Done") { crop.confirm() }
Button("Cancel") { crop.cancel() }
```

## Kotlin

```kotlin
val crop = rememberMediaCrop(aspect = Size(1080, 1920)) { result ->
  if (result == null) dismiss() else scope.launch { photo = media.prepareImage(source, crop = result.crop, rotation = result.rotation, outputSize = Size(1080, 1920)) }
}

MediaCropper(source, crop, shape = MediaCropShape.Rect)
Button(onClick = crop::turnLeft) { Text("Turn") }
Button(onClick = { crop.confirm() }) { Text("Done") }
Button(onClick = crop::cancel) { Text("Cancel") }
```

- `confirm()` gives `MediaCropResult` (`crop` in fractions of the turned photo, `rotation` 0, 90, 180 or 270 clockwise) and calls `onFinish` with it. `cancel()` calls `onFinish` with `nil` (`null`). `onFinish` is called once, a command after that does nothing, and `confirm()` before the photo is shown gives `nil` (`null`).
- `initial` is the first frame (fractions of the photo). It is fitted to the proportions and the limits and keeps its middle. Without it the frame is the largest one in the middle. `reset()` returns to it.
- `turnLeft()` and `turnRight()` turn the photo by a quarter, the frame turns with it. `allowsRotation: false` turns that off. Only turns by 90 degrees exist.
- `shape: .circle` (`MediaCropShape.Circle`) dims the photo outside a circle for an avatar, the result is still a rectangle.
- The source is a local file (`file://`). Anything else, a file that is no image, or a refusal of the memory budget is a `MediaError` in `crop.failure` (`MediaException`). The decode takes its raster from the shared memory budget before it allocates and gives it back when it is done.
- The result passes the core check: the frame is inside the photo, it holds a pixel on both axes and keeps the proportions within one pixel per edge, so `prepareImage` never refuses it.

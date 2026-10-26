# Placeholders

While media loads, a place shows placeholders chosen by MediaLayer. You only declare the source.

| Kind | Chain |
| ---- | ----- |
| Image | BlurHash, used only if the source has one, then the image |
| Video with a cover | The cover until the first frame is rendered, then the video |
| Video without a cover | The first frame, cut from the first chunk and shown without starting playback, then the video |
| Video already cached or ready | Playback starts at once, the cover stays until the first frame is rendered |

Videos never use BlurHash. For a stream, the first frame comes from the first chunk through the local proxy, and playback starts as soon as the player has a buffer. Each step replaces the previous one only when it is ready, so nothing flickers. The cover of a video leaves when its first frame is on screen (`rendered` or `ended`), not when the player is ready, so the picture never has a gap. A page that leaves the screen and returns with the same source keeps its frame, and its cover does not show again. A new source starts with its cover. A video file without a video track renders no frame, so its cover stays.

The core decodes a BlurHash into RGBA with a memory limit: the output, the two cosine tables and the color buffer together may not exceed 16 MiB, whatever the sides are. A larger request is refused with a size error.

The place owns this work for the source it was given. When the url, the BlurHash, the cover or the opened library object changes, the old placeholder goes away at once and a late answer for the old input is dropped. The BlurHash decodes off the main thread, and only the result is published on it.

### Swift

```swift
MediaImage(Avatar(url: url, blurHash: hash))     // BlurHash first, then the image
MediaImage(Avatar(url: url, blurHash: nil))      // empty until the image loads
MediaVideo(MomentVideo(url: url, cover: nil))    // first frame, then video
```

### Kotlin

```kotlin
MediaImage(Avatar(url = url, blurHash = hash))   // BlurHash first, then the image
MediaImage(Avatar(url = url, blurHash = null))   // empty until the image loads
MediaVideo(MomentVideo(url = url, cover = null)) // first frame, then video
```

## Asking for the placeholder

`media.placeholder(source:ready:)` returns the placeholder for a source. Pass `ready: true` once the first frame is on screen (`state.isRendered`). The cache does not change the answer for a remote video. A source counts as cached when its whole entry or its first block is stored, and `media.isCached(source:)` tells it.

### Swift

```swift
let placeholder = await media.placeholder(source: video.item, ready: false)
let cached = await media.isCached(source: video.item)
```

### Kotlin

```kotlin
val placeholder = media.placeholder(video.toItem(), false)
val cached = media.isCached(video.toItem())
```

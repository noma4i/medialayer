# Placeholders

While media loads, a place shows placeholders chosen by MediaLayer. You only declare the source.

| Kind | Chain |
| ---- | ----- |
| Image | BlurHash, used only if the source has one, then the image |
| Video with a cover | The cover while the video is not ready, then the video |
| Video without a cover | The first frame, cut from the first chunk and shown without starting playback, then the video |
| Video already cached or ready | Playback starts at once, no placeholder |

Videos never use BlurHash. For a stream, the first frame comes from the first chunk through the local proxy, and playback starts as soon as the player has a buffer. Each step replaces the previous one only when it is ready, so nothing flickers.

The core decodes a BlurHash into RGBA with a memory limit: the output and the two scratch tables together may not exceed 16 MiB, whatever the sides are. A larger request is refused with a size error.

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

`media.placeholder(source:ready:)` returns the placeholder for a source. A source counts as cached when its whole entry or its first block is stored. `media.isCached(source:)` gives the same answer alone.

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

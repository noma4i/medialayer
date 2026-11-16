# Tone

MediaLayer computes color only after a `tone(source, band)` request or a `WithMediaTone`, `rememberMediaTone`, `subscribeTone` subscription. The core stores the result with the cache file. The app does not start pixel preparation.

| Question | Answer |
| -------- | ------ |
| Who prepares the pixels | Android uses a small software decode in Coil, iOS uses an ImageIO thumbnail. For video the platform extracts a downscaled frame |
| When is the tone ready | BlurHash is available at once. After demand appears the core calls the library callback with the source and notifies subscribers |
| What does a tone hold | An 8 by 12 grid of colors, the average color of the whole frame, the average of a top band, the average of the rest, up to 3 dominant colors |
| Where is it kept | With the cache entry of the source. It survives a restart and goes away with the entry |
| How big is it | 311 bytes per entry |

## Read a tone

`tone(source, band)` answers with the tone of the source, or nil when nothing is known yet. `band` is the share of the height that counts as the top, from just above 0 to 1. A top bar of 15 percent of the frame uses `band` 0.15. The same stored tone answers every `band`.

The answer comes from the best source in this order: pixels of the cache entry, then the BlurHash of the source. A BlurHash tone is marked `blurHash` in `source`, a pixel tone `pixels`. A source without both answers nil, never an invented color.

### Swift

```swift
if let tone = try await media.tone(source: item, band: 0.15) {
    let top = tone.top.color          // Rgb with r, g, b
    let isLight = tone.top.luminance > 0.5
    let accents = tone.dominant       // up to 3, ordered by share
}
```

### Kotlin

```kotlin
val tone = media.tone(item, 0.15f)
if (tone != null) {
    val top = tone.top.color          // Rgb with r, g, b
    val isLight = tone.top.luminance > 0.5
    val accents = tone.dominant       // up to 3, ordered by share
}
```

## React to a new tone

The pixel tone replaces BlurHash after demand appears. A late subscription starts source preparation without the view appearing again. In UI the subscription is held by `WithMediaTone` (Swift) or `rememberMediaTone` (Compose): mount subscribes, unmount cancels, the app does not create a `Task` and does not read the tone again.

### Swift

```swift
WithMediaTone(source: source, band: 0.15) { tone, error in
    Text(tone.map { "\($0.top.color)" } ?? "Waiting")
}
```

### Kotlin

```kotlin
val tone by rememberMediaTone(source, 0.15f)
Text(tone.tone?.top?.color?.toString() ?: "Waiting")
```

Rules of `WithMediaTone` and `rememberMediaTone`:

- The first value arrives after the observation is set: `nil`, BlurHash or pixels. A change during the first read is not lost. Several changes in a row merge into one read.
- A `nil` source removes the subscription and clears the value. Another source replaces the subscription and clears the value of the old one. Another `band` replaces the request while the value stays, and does not start pixel analysis.
- The platform does not store values by URL. The core stores them in a bounded table and in the cache entry.

### Low-level subscription

A consumer outside UI calls `subscribeTone(source, band, listener)`. The listener gets `Option<MediaTone>`: first after the observation is set, then after each change. Keep the subscription while you need updates. Cancel removes the listener and the held read.

## What you do not do

- The app does not decode images for tone. Before preparation the platform asks `tonePixelsNeeded`, then downscales the image to 64 pixels on the longer side.
- Without demand, and when the pixel tone is ready, no preparation runs. The core allows only one preparation per key at a time.
- An error releases the request. When the last subscriber leaves, preparation is cancelled unless a `tone` request is still pending. New demand starts it again.
- You do not store the tone yourself. To keep it with a row of your database, write it into a local field of that row (see the local fields section of the DBlayer models document).
- You do not run the analysis twice for one source: the first pixels of a source win, later ones are dropped.

## API changes

`submitTonePixels` is removed. The library installs one `TonePreparation` through `setTonePreparation` when UI opens. `prepare(source)` returns `TonePixels` and does not keep pixels between calls.

Swift: `subscribeTone(url:listener:)` is replaced by `subscribeTone(source:band:listener:)`. Kotlin: `subscribeTone(url, listener)` is replaced by `subscribeTone(source, band, listener)`. `ToneListener.changed` gets `MediaTone?` instead of `ToneSource`. Pass a `MediaItem` with headers instead of a URL. Manual pairing of `subscribeTone` with a repeated `tone()` is replaced by `WithMediaTone` and `rememberMediaTone`.

The `tone(source:band:)` and `tone(source, band)` signatures stay. Without a pixel result the request starts preparation. Test `MediaPlaces` implementations implement `setTonePreparation`, `tonePixelsNeeded`, `playUrl` (returns a `MediaPlayer`) and `readRoute`.

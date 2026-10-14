# Tone

MediaLayer reads the colors of a media item once and keeps them with the cached file. The app asks for them and never runs the analysis. Use it to tint a bar over a photo or a video so the text stays readable.

| Question | Answer |
| -------- | ------ |
| Who analyses the pixels | MediaLayer, from the pixels the platform already decoded. The app passes nothing |
| When is a tone ready | After the first load of the source, or at once from the BlurHash of the source |
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

A tone can improve after the first answer: a BlurHash tone is replaced by a pixel tone when the image has loaded. `subscribeTone(url, listener)` calls the listener when the stored tone of that source changes. Read the tone again inside the callback. Several changes of one source can arrive as one call. Keep the returned subscription for as long as you need the updates.

### Swift

```swift
final class Bar: ToneListener {
    func changed(source: ToneSource) { /* ask tone(source:band:) again */ }
}
let subscription = media.subscribeTone(url: url, listener: bar)
```

### Kotlin

```kotlin
val subscription = media.subscribeTone(url, object : ToneListener {
    override fun changed(source: ToneSource) { /* ask tone(source, band) again */ }
})
```

## What you do not do

- You do not decode the image or the first video frame for the colors. The platform views of MediaLayer hand the decoded pixels over, scaled to at most 64 points on the long side.
- You do not store the tone yourself. To keep it with a row of your database, write it into a local field of that row (see the local fields section of the DBlayer models document).
- You do not run the analysis twice for one source: the first pixels of a source win, later ones are dropped.

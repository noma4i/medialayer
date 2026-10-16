# Declaring media

You declare what to show and where. MediaLayer does the rest: it picks a free, invisible player from the pool, prepares the source in it and puts it in place when ready, loads and caches images and audio, and prefetches what is nearby.

## Video

### Swift

```swift
struct MomentPage: View {
  let moment: Moment
  let isActive: Bool

  var body: some View {
    MediaVideo(MomentVideo(url: moment.videoUrl, cover: moment.coverUrl),
               playing: isActive,
               muted: Media.muted)
  }
}
```

### Kotlin

```kotlin
@Composable
fun MomentPage(moment: Moment, isActive: Boolean) {
  MediaVideo(
    source = MomentVideo(url = moment.videoUrl, cover = moment.coverUrl),
    playing = isActive,
    muted = Media.muted
  )
}
```

## Image

### Swift

```swift
MediaImage(Avatar(url: user.avatarUrl, blurHash: user.blurHash))
  .frame(width: 44, height: 44)
```

### Kotlin

```kotlin
MediaImage(
  source = Avatar(url = user.avatarUrl, blurHash = user.blurHash),
  modifier = Modifier.size(44.dp)
)
```

An image fills its view by default and is decoded at the size of the view. Pass `contentMode: .fit` (Swift) or `contentScale = ContentScale.Fit` (Kotlin) to show the whole image inside the view.

A load of a task that is already cancelled delivers no image and reports no state. A task that is cancelled in the instant after the image arrived and before the task resumes still gets the image, because Nuke ignores a cancel after its result is final. The image is in the cache, so the view replaces it at the next key or on disappear and nothing is reported as failed. Do not add a second cancel check after the await: no test can reach that instant.

### Swift

```swift
MediaImage(Poster(url: moment.posterUrl), contentMode: .fit)
  .frame(width: 120, height: 160)
```

### Kotlin

```kotlin
MediaImage(
  source = Poster(url = moment.posterUrl),
  modifier = Modifier.size(width = 120.dp, height = 160.dp),
  contentScale = ContentScale.Fit
)
```

## Image in a grid of tiles

A grid of small images (a profile of moments, a gallery) pays for every tile while it scrolls. Pass `weight = MediaWeight.Tile` (Kotlin) to make a tile light. A tile keeps what a correct place needs: one claim per tile (the key stays alive for the Stage while any view of it is on screen, and leaves with the last view), the state reported to the core, the BlurHash while the image loads, the decode at the declared size.

A tile leaves out what a grid does not use:

| A full image does | A tile does not |
| ----------------- | --------------- |
| Reads the view size with `BoxWithConstraints` | Uses `source.size` as the decode size. Without it, Coil takes the size from the layout |
| Submits the pixels of the image for its tone | Submits nothing. `tone(source, band)` answers from the BlurHash of the source only |
| Asks the core for a placeholder again when the image is ready | Asks once, and drops the BlurHash when the image is ready |

Give a tile source its pixel size in `source.size`. Do not use a tile where the app reads the tone of the image or draws a placeholder other than the BlurHash.

Kotlin only (the example is an indented block because Swift has no `weight` yet):

    MediaImage(
      source = MomentThumb(url = moment.thumbUrl, blurHash = moment.blurHash, size = 352 to 352),
      modifier = Modifier.aspectRatio(1f),
      weight = MediaWeight.Tile
    )

Swift has no `weight` yet: the Swift `MediaImage` has no cost measured in a tile grid.

## Video output

A Kotlin `MediaVideo` draws the player on a `TextureView` by default (`output = MediaOutput.Texture`). A texture copies every video frame on the GPU, follows the alpha, clip and translation of its parent, and costs RenderThread time in a scrolling feed. Pass `output = MediaOutput.Window` to draw on a `SurfaceView`: the decoder writes into its own buffer and the compositor places it, with no copy per frame.

A `SurfaceView` does not follow the animations of its parent like a texture does:

| Parent does | `MediaOutput.Texture` | `MediaOutput.Window` |
| ----------- | --------------------- | -------------------- |
| Rounded clip (`clip` with a shape) | Clips the video | Does not clip the video, the corners stay square |
| `alpha`, fade, cross-fade between pages | Fades the video | Video stays opaque while the rest fades |
| Translation, scale, rotation of the pager | Moves the video with the page | Follows the position of the page, can lag one frame during a fling, does not rotate or scale with the parent |
| Order with other views | Draws inside the Compose tree, overlays work | Draws behind the Compose content: the `content` slot draws over it, a `Dialog` or popup is placed by the compositor and can differ |
| Screenshot, `drawToBitmap`, window capture | Contains the video | Contains a hole or a black rectangle |

Do not use `Window` in a pager on Android yet: a measured run on a Pixel 4a (Android 13, 30 swipes of the feed, 5 pairs) gave no gain (median janky frames 53.6 % against 38.6 % with `Texture`, p90 143 ms against 103 ms) and 5 of 10 runs ended with a native crash in `android.view.SurfaceView$SurfaceViewPositionUpdateListener.positionChanged` (libhwui worker thread) while the pager moved the view; the same build with `Texture` did not crash. Keep `Texture` for every pager page. The notes below apply only to a view that is not moved by a pager.

Use `Window` for a full-width feed page that the app does not clip, fade or rotate. Keep `Texture` where the video sits under a rounded clip, an alpha animation or a shared-element transition. The pool is unchanged: the same players, the same claims, the same window. Only the view that shows a player differs.

Kotlin only (the example is an indented block because Swift draws video on an `AVPlayerLayer` and has no such choice):

    MediaVideo(
      source = MomentVideo(url = moment.videoUrl, cover = moment.coverUrl),
      playing = isActive,
      output = MediaOutput.Window
    )

## Audio

### Swift

```swift
MediaAudio(VoiceNote(url: message.audioUrl)) { audio in
  Button(audio.isPlaying ? "Pause" : "Play") { audio.toggle() }
  ProgressView(value: audio.progress)
}
```

### Kotlin

```kotlin
MediaAudio(VoiceNote(url = message.audioUrl)) { audio ->
  Button(onClick = { audio.toggle() }) { Text(if (audio.isPlaying) "Pause" else "Play") }
  LinearProgressIndicator(progress = { audio.progress })
}
```

While an audio plays, its progress updates 4 times per second. A paused audio updates for 1 second after it appears or after a seek, then stops, so a chat full of voice notes does not wake up every 250 milliseconds.

## One-shot video

A video loops by default. Pass `loop: false` to play it once. `onFirstFrame` runs when the first frame is on screen, once per load. `onEnd` runs each time a one-shot video reaches its end. A source with a `file://` URL plays from its own path: it is never fetched, cached or prefetched, and it shows no placeholder.

### Swift

```swift
MediaVideo(IntroClip(url: introUrl), loop: false, onFirstFrame: { hideSplash() }, onEnd: { showHome() })
```

### Kotlin

```kotlin
MediaVideo(
  source = IntroClip(url = introUrl),
  playing = true,
  loop = false,
  onFirstFrame = { hideSplash() },
  onEnd = { showHome() }
)
```

## Video position

`onProgress` receives the played part of a video as a number from 0 to 1: the position divided by the duration. It runs 10 times a second while the video plays. A paused video reports once when it appears or when `playing` changes, then stops, so a paused list does not wake up. A video without a known duration reports nothing. The callback runs on the main thread: do not do heavy work in it.

### Swift

```swift
MediaVideo(VoiceClip(url: clip.url), playing: true, onProgress: { fraction in ring = fraction })
```

### Kotlin

```kotlin
MediaVideo(
  source = VoiceClip(url = clip.url),
  playing = true,
  onProgress = { fraction -> ring = fraction }
)
```

## Audio speed

`rate` sets the speed of an audio place. The default is 1. The library keeps values from 0.25 to 4 and ignores values that are not numbers. `audio.setRate(_:)` on Swift and `audio.rate = ` on Kotlin change it while the place is shown. The rate resets to 1 when the player is reused for another source.

### Swift

```swift
MediaAudio(VoiceNote(url: message.audioUrl), rate: 1.5) { audio in
  Button("2x") { audio.setRate(2) }
}
```

### Kotlin

```kotlin
MediaAudio(VoiceNote(url = message.audioUrl), rate = 1.5f) { audio ->
  Button(onClick = { audio.rate = 2f }) { Text("2x") }
}
```

## Parameters

| Parameter | Kinds | Meaning |
| --------- | ----- | ------- |
| source | all | A generated type from your `.mdl` file with the URL and optional cover or BlurHash |
| `playing` | video, audio | True to play, false to stay prepared and paused |
| `muted` | video, audio | True to silence this place. `Media.muted` silences the whole app. The `silent` option of `open` overrides it |
| `loop` | video | True (default) repeats the video. False plays it once and ends in the `ended` state |
| `onFirstFrame` | video | Runs when the first frame is on screen. Once per load |
| `onEnd` | video | Runs each time a one-shot video reaches its end |
| `rate` | audio | Playback speed, 0.25 to 4, default 1 |
| content closure | all | Optional. Receives the `MediaState` to draw your own loading and error views |

## State

All three kinds share one state type, `MediaState`:

| State | Meaning |
| ----- | ------- |
| `idle` | Nothing requested yet |
| `loading` | Fetching or preparing; placeholders show |
| `ready` | Ready to play |
| `rendered` | Video only. The first frame is on screen |
| `ended` | Video only. A one-shot video reached its end. Playing it again returns to `rendered` |
| `failed` | The load failed; carries the reason |

`rendered` and `ended` count as ready: `state.isReady` is true for `ready`, `rendered` and `ended`. A ready place never goes back to `loading` by itself.

### Swift

```swift
MediaVideo(source, playing: isActive) { state in
  if case .failed = state { RetryButton() }
}
```

### Kotlin

```kotlin
MediaVideo(source, playing = isActive) { state ->
  if (state is MediaState.Failed) RetryButton()
}
```

## Playing and sound

Every place takes `playing` and `muted`. A player that is not playing stays prepared and paused. `Media.muted` mutes the whole app. Muting changes only the volume of the player; it never touches the audio session, so another app's music is not interrupted by a mute toggle.

A pager declares the neighbour page that the finger is dragging into with `peeking: true`. The player of that page plays muted beside the page that plays, so the drag shows a moving picture. When the page becomes `playing` its player keeps running and gets its sound, with no restart. When `peeking` ends without `playing`, the player pauses. One page peeks at a time, and a page that plays never peeks.

By default MediaLayer never sets up or activates the audio session, and your app chooses the category. Pass an `audioFocus` policy to `open` and MediaLayer owns it instead: [audio focus](audio-focus.md).

You report only whether a place is visible or active. Prefetch, preroll, handoff and release follow from that. Several views can show the same url at once (a chat row and its message preview). The place keeps its state until the last of them reports that it left, so a view that stays on screen never falls back to idle because another view of the same url disappeared.

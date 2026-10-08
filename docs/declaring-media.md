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

MediaLayer never sets up or activates the audio session. Your app chooses the category and activates it (`AVAudioSession` on iOS); on Android Media3 asks for audio focus when a player starts.

You report only whether a place is visible or active. Prefetch, preroll, handoff and release follow from that.

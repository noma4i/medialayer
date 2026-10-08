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

## State

All three kinds share one state type, `MediaState`: `idle`, `loading`, `ready`, `failed`.

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

You report only whether a place is visible or active. Prefetch, preroll, handoff and release follow from that.

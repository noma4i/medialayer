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

On iOS a GIF animates and decodes its frames only while its view is on screen and the app is in the foreground. A view that is scrolled out of its scroll view, hidden, removed from its window or covered by a full screen is off screen. A transparent screen over the view does not stop it. Two views of one url and size share one frame decode, and the decode stops only when the last of them leaves. You declare nothing for this.

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

## Image as a value

To get the decoded image without a view (a share card drawn by `ImageRenderer`, a bitmap for a canvas), call `image` on the media context. It goes through the same loader, the same core cache, the same item state and tone as `MediaImage`, and it does not wait for a window or a lifecycle. It returns `nil` (`null`) when the source fails or the task is cancelled, and a second call for the same source and size reads the cache.

### Swift

```swift
let avatar = await context.image(for: Avatar(url: user.avatarUrl), points: CGSize(width: 44, height: 44), scale: 3)
```

### Kotlin

```kotlin
val avatar = media.image(context, Avatar(url = user.avatarUrl), width = 44f, height = 44f, density = 3f)
```

Swift: `MediaContext.image(for:points:scale:)`. Kotlin: `MediaLayer.image(context, source, width, height, density)` is a `suspend` function. Width and height are in points (dp). Shared `imagePixelSize` derives decode dimensions from placement and display scale, with placement rounded up to multiples of 8. `source.size` describes source dimensions and caps decoding without upscaling. Crop preserves the source ratio while covering placement, and Fit preserves it while fitting inside placement. Each dimension is limited to 16384 pixels. A 1080x1920 source in a 352x352 placement decodes at 352x626 for Crop or 198x352 for Fit. The app must not call this export method for a list of views: use `MediaImage` there.

## Изображения в списках и сетках

Set `source.size` to the source dimensions in pixels when available.
With declared dimensions, Kotlin `MediaImage` uses `Box` and Coil's placement size resolver.
Without declared dimensions, it obtains placement constraints through `BoxWithConstraints` and computes pixels through `imagePixelSize`.

Read image work explicitly with Kotlin `media.imageStats().snapshot()` or Swift `media.imageStats().snapshot()`.
The snapshot counts requests, decodes, memory hits, disk hits, decoded pixels, placement pixels, bitmap bytes, active requests, active observers, and cancellations (one total, not per URL).
Bitmap bytes accumulate decoded allocations, rather than retained memory.
Counters update on image events and perform no work while idle.
`MediaImage.request` and `MediaImage.decode` identify Android async trace sections and Swift signpost intervals.
`fetchImage` returns `ImageFetchResult`, containing `bytes` and `source` (`Cache`, `Network`, or `Local`).
Disk hits count results that the core served from its cache, excluding local files and memory hits.

Все изображения сообщают состояние ядру и передают пиксели для вычисления тона.
BlurHash загружается только для видимого изображения до `Ready`.
После `Ready` библиотека убирает BlurHash без повторного запроса заглушки.

Видимость определяет библиотека, приложение не передаёт её параметром.
До размещения, вне окна и при остановленной Activity запрос Coil не загружает и не декодирует изображение.
Уход из окна или состояния `STARTED` отменяет текущую загрузку и снимает заявку видимости.
Вход в окно возобновляет отменённую загрузку.
Изображения с общим URL сохраняют заявку ядру до ухода последнего видимого изображения.

Пример Kotlin:

    MediaImage(
      source = MomentThumb(url = moment.thumbUrl, blurHash = moment.blurHash, size = 352 to 352),
      modifier = Modifier.aspectRatio(1f)
    )

Swift `MediaLayerUI` не имеет параметра `weight`.
Стоимость Swift `MediaImage` в сетке требует отдельного измерения.

## Video output

A Kotlin `MediaVideo` uses `TextureView` by default (`output = MediaOutput.Texture`).
The experimental opt-in is `output = MediaOutput.Window`, using `SurfaceView` on every supported API (minSdk 33) when `alpha == 1f`.
When the explicit `alpha` parameter differs from 1, it selects Texture and records the reason in Trace.
No second flag or output enum value is required.
Android and Media3 provide no estimate of saved frames for this application.

| Behavior | `MediaOutput.Texture` | `MediaOutput.Window` |
| --- | --- | --- |
| `cornerRadius = 16.dp` | Compose rounded clip | Public `ViewOutlineProvider` and `clipToOutline`, experimental visibility requires device verification |
| Explicit `alpha`, fade, cross-fade | Compose opacity | Selects Texture while `alpha != 1f` |
| Parent clip or alpha | Follows Compose drawing | Not inspected, pass the explicit parameters or retain Texture |
| Pager translation | Follows the page | Uses platform position updates, fling alignment and stability require device verification |
| Rotation, shared-element effects | Follows Compose drawing | Not qualified, retain Texture |
| Overlays and transparent backgrounds | Draws within Compose | Default Z-below ordering, test overlapping pages, controls and dialogs |
| View bitmap capture | Includes video | Can omit video, use compositor capture or PixelCopy and verify the result |

Window retains `ContentFrame` while its player changes or becomes null.
Texture creates `ContentFrame` only while its player is non-null, including fallback from Window.
Defaults `cornerRadius = 0.dp` and `alpha = 1f` add neither clip nor opacity modifiers.
It uses attachment-based `SurfaceView` lifetime, without moving a View between pager parents.
Different composed pages still own different Views.
Slots, claims, playback, peeking and the pool window use the same implementation for both outputs.

Rounded Window output is experimental: visible rounding is not proven without a device run.
Keep Texture in production until the [device procedure](video-output-measurement.md) passes.
Switching between Window and Texture during a fade changes the output View and requires first-frame and black-rectangle checks.
For continuous cross-fades, choose Texture for the entire transition, including opaque endpoints.

An earlier Pixel 4a Android 13 experiment crashed in `SurfaceViewPositionUpdateListener.positionChanged` during pager motion in 5 of 10 runs.
Its median janky frames were 53.6% with Window versus 38.6% with Texture across 30 swipes and 5 pairs.
Window on API 33 was checked on a Pixel 4a by the V3 Android coordinator.
Media3 1.11.1 retains its API 34 Compose size synchronization fix for issue #1237.

Kotlin only (the example is an indented block because Swift draws video on an `AVPlayerLayer` and has no such choice):

    MediaVideo(
      source = MomentVideo(url = moment.videoUrl, cover = moment.coverUrl),
      playing = isActive,
      output = MediaOutput.Window,
      cornerRadius = 16.dp,
      alpha = 1f
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

A seek by fraction chosen before the audio loads is applied when the player reports ready. A stream with no known duration at that moment keeps the fraction until the next player event that reports a duration.

Audio does not depend on the size or position of its view. Progress, seek and the end of the audio work in a 0 x 0 host and in a view that is off screen. Only the app going to the background pauses the progress updates.

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
| `group` | video, audio | Optional name of the place group, for example `chat`. A playback suspension can allow a group and keep its places playing |
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

When the app loses its audio output (headphones unplugged on iOS, `ACTION_AUDIO_BECOMING_NOISY` on Android), `open` has already set `Media.muted` to true. The players do not pause, and the app calls nothing. The app turns the sound on again by setting `Media.muted = false`.

## Suspending playback

A dialog, a share sheet or a modal screen stops all players while it is open. Call `suspendPlayback` on the `MediaLayer`. It returns a `PlaybackSuspension`. While at least one suspension is alive, every player is paused, except the places whose `group` is in the `allowGroups` of every live suspension. Places stay mounted and prepared.

`release()` acts once, however often you call it. Swift also releases when the object is freed. In Kotlin `PlaybackSuspension` is `AutoCloseable` and `close()` releases it. You can release from any thread. When the last suspension is released, the places that played before play again. A place with manual resume (a chat voice note that lost the election) stays idle. A blank group in `allowGroups` is an error.

### Swift

```swift
let suspension = try media.suspendPlayback(allowGroups: ["chat"])
// ... dialog is open
suspension.release()
```

### Kotlin

```kotlin
media.suspendPlayback(listOf("chat")).use {
  // ... dialog is open
}
```

MediaLayer owns the sound policy of the players: you pass an `audioFocus` policy to `open` (default `mix`), and the app writes no audio session or audio focus code for playback: [audio focus](audio-focus.md).

You report only whether a place is visible or active. Prefetch, preroll, handoff and release follow from that. Several views can show the same url at once (a chat row and its message preview). The place keeps its state until the last of them reports that it left, so a view that stays on screen never falls back to idle because another view of the same url disappeared.

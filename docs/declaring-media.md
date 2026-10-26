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

## Sources without a class

`MediaSourceValue` is a source declared by value, next to the generated types of the `.mdl` file. The app writes no class and no string glue. The constructors and factories keep the text as declared and call nothing native. The core cleans it once when the declaration enters its input queue, off the main thread: the url loses its edge whitespace, a blank cover or blur hash is absent.

### Swift

```swift
MediaSourceValue.file(recording.fileUrl, kind: .video)                         // path or file URL
MediaSourceValue.remote(user.avatarUrl, kind: .image, blurHash: user.blurHash)  // http or https text or URL
MediaSourceValue.bundled("welcome", extension: "m4v", kind: .video)            // bundle resource, Bundle.main by default
MediaSourceValue(url: row.url, kind: .image, size: MediaSize(width: 288, height: 288), strategy: .whole)
```

### Kotlin

```kotlin
MediaSourceValue.file(recording.file, Kind.VIDEO)                              // path, File or file Uri
MediaSourceValue.remote(user.avatarUrl, Kind.IMAGE, blurHash = user.blurHash)
MediaSourceValue.raw(context, R.raw.welcome, Kind.VIDEO)                       // res/raw
MediaSourceValue.asset("welcome.m4v", Kind.VIDEO)                              // assets
MediaSourceValue(row.url, Kind.IMAGE, size = 288 to 288, strategy = CacheStrategy.WHOLE)
```

- A path or a `file://` text becomes one canonical `file://` url. A relative path, a web url or an empty text gives an empty url. A web url stays only when it is a valid `http` or `https` text with a host, anything else gives an empty url.
- A source with an empty url, or a text the core refuses (`remote` with another scheme, `file` with a relative path), is still created. Its place fails with `MediaState.failed` (`MediaState.Failed`) and the detail `the source has no url`. No player, no prefetch, no request starts. The place leaves the failed state when the last view of it goes away. A missing bundle resource gives the same result, so the app has no `nil` branch to write.
- `bundled`, `raw` and `asset` take a video or an audio. An image is a programming error: Swift stops with `preconditionFailure`, Kotlin throws `IllegalArgumentException`, both with the text `a bundled source is a video or an audio, not an image` from the core (`bundled_source_error`, asked only on this error path).
- iOS reads a bundle resource as a `file://` url. Android `res/raw` is `android.resource://<package>/<id>` and `assets` is `asset:///<name>`. ExoPlayer reads both itself, the core treats them like `file://`: the source counts as cached, stays out of the prefetch window, goes without the proxy and has an empty placeholder (`is_local_url` is the only list of these schemes). `fetch_image` reads only `file://`.
- `looping` is not a field of a source. `MediaVideo(loop:)` sets it.

## Preview until the upload is ready

`preview` puts a second source in the place of one image. It shows while the main source has no frame, usually a local `file://` copy of a picture that is still uploading. The first frame of the main source replaces it in the same frame, with no empty frame and no timer. The preview takes no place of its own and is never prefetched. The cache key of the main source stays its URL.

### Swift

```swift
MediaImage(Upload(url: message.remoteUrl), preview: message.localPreview.map { Upload(url: $0) })
  .frame(width: 120, height: 160)
```

### Kotlin

```kotlin
MediaImage(
  source = Upload(url = message.remoteUrl),
  modifier = Modifier.size(width = 120.dp, height = 160.dp),
  preview = message.localPreview?.let { Upload(url = it) }
)
```

Without a preview, call `MediaImage` as before. Swift takes the preview as an optional source in a second initializer.

## Image size

`imageSize` answers the pixel size of an image before the app draws it, for a chat bubble or a placeholder frame. The core reads the size from the head of the file: PNG, GIF, WebP and JPEG, and a JPEG turned by EXIF orientation 5 to 8 reports the turned size. A `file://` source is read from its file. A remote source needs one `Range` request for its first 1 MiB (the first block of the cache) and never downloads the whole file; a file that is already stored is read from the cache. The answer is kept with the cache entry, survives a restart and leaves with the entry. A source without an entry keeps it in a bounded table of 256 answers. A repeat for the same source makes no network request. It returns `nil` (`null`) when the head holds no known image.

### Swift

```swift
let size = try await media.imageSize(Upload(url: message.remoteUrl))   // ImageSize(width:height:) or nil
```

### Kotlin

```kotlin
val size = media.imageSize(Upload(url = message.remoteUrl))   // ImageSize(width, height) or null
```

## Media info

`mediaInfo` answers the length of a local video or audio file and the frame of a video, without a player and without loading the file into memory. It reads `file://`, a bundle resource, `android.resource://` and `asset:///` sources with `AVURLAsset` (Swift) and `MediaMetadataRetriever` (Kotlin), off the main thread, and stops when the task is cancelled. `durationMs` is in milliseconds. `frame` is the size as the screen shows it: a video turned by 90 or 270 degrees reports the sides swapped (one rule in the core, `displayFrameSize`). An audio file has a length and no frame.

It returns `nil` (`null`) when the file holds no media, and for a remote source: a remote file has no metadata without a download, and the library never downloads it a second time for this call. A missing or unreadable local file throws `MediaError.Fetch` (`MediaException.Fetch`), as `imageSize` does.

### Swift

```swift
let info = try await media.mediaInfo(MediaSourceValue.file(clipUrl, kind: .video))   // MediaInfo(durationMs:frame:) or nil
```

### Kotlin

```kotlin
val info = media.mediaInfo(MediaSourceValue.file(clipFile, Kind.VIDEO))   // MediaInfo(durationMs, frame) or null
```

## Preparing media for sending

`prepareVideo` and `prepareImage` make a local file ready to upload. Both take a `file://` source and run off the main thread. Cancelling the call stops the work and leaves no output file.

```swift
let video = try await media.prepareVideo(source, limits: defaultVideoLimits(), progress: { fraction in })
let photo = try await media.prepareImage(source, quality: 0.9)
```

```kotlin
val video = media.prepareVideo(source, limits = defaultVideoLimits(), onProgress = { fraction -> })
val photo = media.prepareImage(source, quality = 0.9)
```

- The result is `PreparedMedia`: the file and `isNew`. When `isNew` is false the source already fitted, carries no private metadata and is returned untouched, so there is nothing to delete.
- Video limits are yours: `VideoLimits` holds the long side, the bitrate corridors, the frame rates and the audio settings. `defaultVideoLimits()` gives a long side of 1920, corridors from 0.5 to 3.5 Mbit/s and 30 to 60 frames per second.
- The video is made upright, with even sides, a long side up to your table (never scaled up), a bitrate inside the corridor of the result and a limited frame rate. The codecs are H.264 and AAC in an mp4 file made for streaming.
- The video never carries the metadata of its source: no location or GPS, no creation time, no device or software, no tags. A file that already fits but carries any of them is copied without encoding the video or the audio and without the metadata, and `isNew` is true. The core decides from the keys the platform reads (`videoMetadataToStrip`), only the container keys such as `major_brand` and `handler_name` are allowed.
- The photo is turned upright by its EXIF orientation and saved as JPEG with your quality (0.0 to 1.0). No metadata is copied, so the editor tag and the EXIF fields are gone.
- Android reports video progress only while the export runs, with no timer at rest.

`prepareImage` also edits the photo. `maxSize` fits the result into a rectangle and keeps the proportions, it never enlarges unless `enlarge` is true: then a smaller photo grows to the rectangle (540x960 into 1080x1920 gives 1080x1920), the pixel limit of 16384 per side still applies. `rotation` turns the upright photo clockwise by a multiple of 90 degrees (a negative value turns it the other way, any other value throws `MediaError.Fetch`), it joins the EXIF turn in one turn. `crop` is a rectangle in fractions from 0 to 1 of the turned photo (the one the user sees, see `docs/crop.md`), the origin is the top left corner, a crop past the edge is held inside the photo. `outputSize` gives the exact size and wins over `maxSize`. `mirror` flips the result horizontally, for a front camera. The core plans the sizes in the order turn, crop, mirror, fit. An edit that holds no pixel, such as an empty crop or a size of zero, throws `MediaError.Fetch` (`MediaException.Fetch`).

The photo is decoded no larger than the edit needs. When the crop is larger than the output, the core plans a smaller decode (Android `inSampleSize` as a power of two, iOS an exact long side) that keeps every side of the crop at or above the output, so an 8160x6120 photo cut to 1080 pixels is decoded at about 2040x1530 and not at 50 megapixels. A crop at or below the output size is decoded at full size. The memory asked of the Ledger is the decoded raster, and the limit of 16384 per side still applies to the file.

```swift
let chat = try await media.prepareImage(source, quality: 0.7, maxSize: CGSize(width: 1080, height: 1920))
let story = try await media.prepareImage(source, maxSize: CGSize(width: 1080, height: 1920), enlarge: true)
let avatar = try await media.prepareImage(source, crop: CGRect(x: 0.1, y: 0.2, width: 0.8, height: 0.45), outputSize: CGSize(width: 1080, height: 1080))
let selfie = try await media.prepareImage(source, mirror: true)
```

```kotlin
val chat = media.prepareImage(source, quality = 0.7, maxSize = Size(1080, 1920))
val story = media.prepareImage(source, maxSize = Size(1080, 1920), enlarge = true)
val avatar = media.prepareImage(source, crop = RectF(0.1f, 0.2f, 0.9f, 0.65f), outputSize = Size(1080, 1080))
val selfie = media.prepareImage(source, mirror = true)
```

## Encoding an image in memory

`encodeImage` saves an image that is already in memory as a JPEG or PNG file, or encodes the bytes of an image (WebP for example) as JPEG or PNG bytes. The image is turned upright by its orientation, a JPEG has no transparency so it is flattened onto white, `quality` (0.0 to 1.0) is for JPEG only. Both calls take their memory from the shared budget and run off the main thread.

```swift
let draft = try await media.encodeImage(uiImage, format: .jpeg, quality: 0.9)   // PreparedMedia, a file
let png = try await media.encodeImage(bytes: webpBytes, format: .png)           // Data
let size = MediaImageDecode.imageSize(data: webpBytes)                            // ImageSize or nil
```

```kotlin
val draft = media.encodeImage(bitmap, ImageFormat.JPEG, quality = 0.9)   // PreparedMedia, a file
val png = media.encodeImage(webpBytes, ImageFormat.PNG)                  // ByteArray
val size = MediaImageDecode.imageSize(webpBytes)                         // ImageSize or null
```

`MediaImageDecode.imageSize` reads the size from the header, with the EXIF turn applied, so it equals the size of the decoded image.

## Decoding a local picture

`MediaImageDecode.decode` turns a local picture into an upright image for a picker, a crop screen or an editor. The EXIF orientation is applied, the long side is cut to `maxSide` (nil or null keeps the size) and a picture is never enlarged. The decode runs off the main thread and takes its memory from the same budget as the players. When the budget refuses, the call throws `MediaError` (Swift) or `MediaException` (Kotlin). Without an open MediaLayer the call throws `Closed`.

```swift
let photo = try await MediaImageDecode.decode(file: fileURL, maxSide: 2048)
let fromBytes = try await MediaImageDecode.decode(data: bytes, maxSide: 2048)
```

```kotlin
val photo = MediaImageDecode.decode(file, maxSide = 2048)   // also decode(uri, maxSide) and decode(bytes, maxSide)
```

The size of a local file without decoding it is `imageSize` on a `file://` source: it reads the head of the file and applies the EXIF turn.

`MediaImageLoad.loadBitmap` loads a picture by URL straight into an upright image of the wanted side. With `circle` the result is a square of `side` pixels cut to a circle, for an avatar. It works without an open MediaLayer.

```swift
let avatar = try await MediaImageLoad.loadBitmap(url: avatarURL, directory: cacheDirectory, side: 96, circle: true)
```

```kotlin
val avatar = MediaImageLoad.loadBitmap(avatarUrl, cacheDirectory, side = 96, circle = true)
```

## Recording a voice message

`MediaVoiceRecorder` records the microphone into an m4a file (AAC at 44.1 kHz, one channel, 128 kbit/s). `start` takes the audio focus for recording from `Media.audio` and `stop` gives it back. `levels` gives the wave level from 0 to 1 while the recording runs, from one formula of the core, and nothing at rest. The recording ends by itself at the longest length (default 10 minutes, set between 1 second and 1 hour). A refused microphone permission, a busy microphone and a recording without sound throw `MediaError.Voice` (Swift) or `MediaException.Voice` (Kotlin) with the same text on both platforms.

```swift
let recorder = MediaVoiceRecorder(maxDuration: 120)
try await recorder.start(to: fileURL)
for await level in recorder.levels { draw(level) }
let voice = try await recorder.stop()   // VoiceRecording(url, durationMs)
```

```kotlin
val recorder = MediaVoiceRecorder(context, maxDurationMs = 120_000)
recorder.start(file)
recorder.levels.collect { level -> draw(level) }
val voice = recorder.stop()   // VoiceRecording(file, durationMs)
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
`prepareImage` adds the raster it actually decodes to the same counters (one decode, width times height of the sampled raster, not the source), so a 50 MP photo prepared for a 1080 side shows about 1 MP in `decodedPixels`.
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

A Kotlin `MediaVideo` always draws through a `TextureView`. There is no output parameter.
A measured feed of six video pages on a Pixel 4a dropped 22.19% of frames on `SurfaceView` against 11.45% on `TextureView`, and a feed of one video showed no difference.
`cornerRadius`, `alpha`, parent clips, pager translation, rotation and shared-element effects follow Compose drawing.
`ContentFrame` exists only while the page has a player.
Defaults `cornerRadius = 0.dp` and `alpha = 1f` add neither clip nor opacity modifiers.
Different composed pages own different Views.
Swift draws video on an `AVPlayerLayer` and has the same single path.

Kotlin only (the example is an indented block because Swift has no such parameters):

    MediaVideo(
      source = MomentVideo(url = moment.videoUrl, cover = moment.coverUrl),
      playing = isActive,
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

`playing` starts the audio when it is true and stays false by default. Both platforms apply a change of `playing` to the mounted audio: the same `audio` object plays or pauses, and nothing is mounted again. While `playing` keeps its value, a rerender, a new `rate`, `muted` or `group` leave the buttons alone: a local `toggle()` stays in force, and so do `play()` and `pause()` on Kotlin. A new source starts a new `audio` that follows the current `playing`.

While an audio plays, its progress updates 4 times per second. A paused audio updates for 1 second after it appears or after a seek, then stops, so a chat full of voice notes does not wake up every 250 milliseconds.

`onEnd` runs each time the audio reaches its end, once per end. At the end the library rewinds the audio to the start and pauses it: `isPlaying` is false and `progress` is 0. A new `play` starts from the beginning. The app does not rewind or pause the audio itself.

`isSounding` is true while the audio plays and the player is ready or rendered. `isBuffering` is true while the audio is set to play and the player is idle or loading. `MediaState.isFailed` (Swift and Kotlin) is true for the failed state. The app does not write these checks.

A seek by fraction chosen before the audio loads is applied when the player reports ready. A stream with no known duration at that moment keeps the fraction until the next player event that reports a duration.

A source without a mounted view seeks by key: `MediaLayer.seek(key, fraction)` on Kotlin and `seek(key:fraction:)` on `MediaPlaces` on Swift. The key is the source URL. The call works for any key, not loaded or paused too, and the position is applied when the key loads. A call made after MediaLayer closed, or while the input is full, throws and the caller sees the error. Several calls for one key before the next event keep the last fraction. A fraction outside 0..1 is clamped, a NaN or infinite fraction is ignored. `MediaPlace.seek` of a mounted place works as before.

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
| `muted` | video, audio | True to silence this place. `Media.muted` silences the whole app. On a simulator or emulator the library keeps all players muted and this value does not unmute them |
| `group` | video, audio | Optional name of the place group, for example `chat`. A playback suspension can allow a group and keep its places playing |
| `loop` | video | True (default) repeats the video. False plays it once and ends in the `ended` state |
| `onFirstFrame` | video | Runs when the first frame is on screen. Once per load |
| `onEnd` | video, audio | Runs each time a one-shot video or an audio reaches its end. An audio is already rewound to the start and paused |
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
| `ended` | A one-shot video or audio reached its end. The core stops it and rewinds it to the start. The next play starts from zero and returns the place to `rendered`, no manual seek is needed. A seek made after the end is kept |
| `failed` | The load failed; carries the reason |

`rendered` and `ended` count as ready: `state.isReady` is true for `ready`, `rendered` and `ended`. A ready place never goes back to `loading` by itself. `onFirstFrame` fires once per source of a place: a pause, a late `loading` or a reload does not repeat it, and only a new source (or the view leaving the screen) resets it, so the app needs no "started" flag of its own.

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

Every place takes `playing` and `muted`. A player that is not playing stays prepared and paused. `Media.muted` mutes the whole app. In Swift read it and change it with `try Media.setMuted(_:)`: the flag changes only after the core took the mute, a refusal throws and `Media.failure` keeps the reason. In Compose assign `Media.muted`; a refusal of the core shows in `Media.failure`. A place that the core refuses to open shows `MediaState.failed`. Muting changes only the volume of the player; it never touches the audio session, so another app's music is not interrupted by a mute toggle.

A pager declares the neighbour page that the finger is dragging into with `peeking: true`. The player of that page plays muted beside the page that plays, so the drag shows a moving picture. When the page becomes `playing` its player keeps running and gets its sound, with no restart. When `peeking` ends without `playing`, the player pauses. One page peeks at a time, and a page that plays never peeks.

When the app loses its audio output (headphones unplugged on iOS, `ACTION_AUDIO_BECOMING_NOISY` on Android), `open` has already set `Media.muted` to true. The players do not pause, and the app calls nothing. The app turns the sound on again by setting `Media.muted = false`.

## Suspending playback

A dialog, a share sheet or a modal screen stops all players while it is open. Call `suspendPlayback` on the `MediaLayer`. It returns a `PlaybackSuspension`. While at least one suspension is alive, every player is paused, except the places whose `group` is in the `allowGroups` of every live suspension. Places stay mounted and prepared.

A place without a `group` belongs to the default group `ring`, so a url played by a place without a group and a place in `system` plays under a suspension `[ring]`. A place can change its `group` while it is mounted: it starts or stops with the next change. Several places can show the same url with different groups. That url plays when one of their groups is allowed by every live suspension, and the groups of two suspensions never add up (`[system]` and `[ring]` together allow neither). `release()` acts once, however often you call it. Swift also releases when the object is freed. In Kotlin `PlaybackSuspension` is `AutoCloseable` and `close()` releases it. You can release from any thread. When the last suspension is released, the places that played before play again. A place with manual resume (a chat voice note that lost the election) stays idle. A blank group in `allowGroups` is an error.

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

### Declaring a suspension in the view tree

A screen that stops playback while it is shown declares it. Swift: `.mediaPlaybackSuspended(active:allowGroups:)` on any view. Kotlin: `SuspendMediaPlayback(active, allowGroups)` in the composition. The declaration takes one suspension when it is mounted and `active` is true. A change of `allowGroups` replaces the suspension: the new one is taken before the old one is released, so no player starts in between. `active = false`, or the view leaving, releases it. The same parameters again take nothing. Until the library is open the declaration holds nothing and takes the suspension when it opens. A refusal of the core shows in `Media.failure`. The app writes no `suspendPlayback` call, no wait for the open and no lifecycle code.

Going to the background pauses the playing places and returning plays them again. The app does not put the lifecycle state into `playing`. The app still decides the reasons (dialog, share sheet, modal) and the `allowGroups`.

#### Swift

```swift
ChatScreen()
  .mediaPlaybackSuspended(active: isShareSheetOpen, allowGroups: ["chat"])
```

#### Kotlin

```kotlin
SuspendMediaPlayback(active = isShareSheetOpen, allowGroups = listOf("chat"))
```

MediaLayer owns the sound policy of the players: you pass an `audioFocus` policy to `open` (default `mix`), and the app writes no audio session or audio focus code for playback: [audio focus](audio-focus.md).

You report only whether a place is visible or active. Prefetch, preroll, handoff and release follow from that. Several views can show the same url at once (a chat row and its message preview). The place keeps its state until the last of them reports that it left, so a view that stays on screen never falls back to idle because another view of the same url disappeared.

# Changelog

## 3.22.0 - 2026-10-26

### Added
- `MediaCrop.snapshot()` gives the final scale, move and change flag of the frame now, and `cancel()` returns the same.

### Changed
- `onPositionChange` receives a `CropGestureReport` with the kind, duration, largest move, scale and move changes and the final transform.

## 3.21.0 - 2026-10-26

### Added
- `MediaCrop` reports gestures through `onPositionChange` and `MediaCropResult` carries the final scale, move and change flag.

## 3.20.0 - 2026-10-26

### Added
- `prepareImage` counts the raster it decodes in `imageStats().snapshot()` (`decodes`, `decodedPixels`, `bitmapBytes`) on iOS and Android.

## 3.19.0 - 2026-10-26

### Added
- `CameraState.refusedMs` gives the length of a recording refused as too short.

### Changed
- `MediaError.Camera` carries `recordedMs`, the length of a recording that is too short.

## 3.18.1 - 2026-10-26

### Changed
- The 3.8.0 notes list the private slot commands, async `create` and removed `replaceImageWindow`.

## 3.18.0 - 2026-10-26

### Fixed
- `prepareVideo` never returns a file with the location, creation time, device or tags of its source: a file that fits but carries them is copied without encoding and without them (`isNew` is true), a clean file stays as it is. The core names such keys with `videoMetadataToStrip`.

## 3.17.0 - 2026-10-26

### Added
- `MediaCropper` lets the user move and scale a photo crop frame with fixed or free proportions on iOS and Android, and `prepareImage` takes the result as `crop` and `rotation`.

## 3.16.2 - 2026-10-26

### Fixed
- A `MediaImage` or `MediaVideo` whose ancestor moves without a layout pass (offset, transform, layer position, bounds origin, `clipsToBounds`, hidden layer) now reports its first visibility on iOS without a touch.

### Added
- `ML_TRACE` and the Android trace print `view onScreen <key tail> <true|false>` and `image load <key tail> start|done|failed`.

## 3.16.1 - 2026-10-26

### Fixed
- `prepareImage` decodes a photo at the size its crop and output ask for, not at the full size of the file: a 50 MP photo cut to 1080 pixels holds under a third of its pixels, and the Ledger is asked for the decoded raster.

## 3.16.0 - 2026-10-26

### Added
- `CaptureProfile.stabilization` steadies the recorded video: `.circle` is cinematic, `.moment` and `.photo` are off.

## 3.15.0 - 2026-10-26

### Added
- `MediaCamera` takes photos and records video with a preview on iOS (`AVCaptureSession`) and Android (CameraX). The app declares a profile (`.photo`, `.moment`, `.circle` or its own) and a `MediaCameraPreview`. The camera runs only while the preview is on screen and the app is in front, and the library releases the camera, the microphone focus and the memory budget by itself.
- The core decides the format of the camera, the limits and verdicts of a recording, the zoom, the flash, the one restart after a reset of the camera service and the state of the camera (`capture_step`).
- A photo is an upright JPEG, a clip carries its length and frame. `MediaError.Camera` names the reason: permission, no camera, busy, too short, memory.
- `observeFrames` gives the frames of the preview to the app for a face search.

## 3.14.0 - 2026-10-26

### Added
- `prepareImage` takes `enlarge`: with `maxSize` a smaller photo is fitted up into the rectangle with its proportions kept (540x960 into 1080x1920 gives 1080x1920). The default stays without enlarging.

## 3.13.0 - 2026-10-26

### Added
- `prepareImage` takes `maxSize` (fits the result into a rectangle, never enlarges), `crop` (a rectangle in fractions of the upright photo), `outputSize` (the exact size) and `mirror` (flips the result horizontally). The core plans every size, the order is turn, crop, mirror, fit. An edit that holds no pixel is refused with `MediaError.Fetch` (`MediaException.Fetch`).
- `MediaLayer.encodeImage` saves an image in memory (Swift `CGImage` or `UIImage`, Kotlin `Bitmap`) as a JPEG or PNG file, and encodes the bytes of an image (such as WebP) as JPEG or PNG bytes. Both take their memory from the shared budget, a JPEG flattens transparency onto white.
- `MediaImageDecode.imageSize` gives the size of an image in memory, with the EXIF turn applied, from its header.
- Core `photo_plan` and `image_encode_plan`.

## 3.12.1 - 2026-10-26

### Added
- Trace lines `stage suspend` and `stage resume` name the playback suspensions that hold the groups.

### Fixed
- Voice recordings use AAC at 44.1 kHz, mono, 128 kbit/s.

## 3.12.0 - 2026-10-26

### Added
- `MediaImageDecode.decode` decodes a local file, a `Uri` (Android) or bytes into an upright `Bitmap` (Swift `PlatformImage`): the EXIF turn is applied, the long side is cut to `maxSide`, the work runs off the main thread and takes its raster from the shared memory budget. A refusal is a `MediaException` (Swift `MediaError`).
- `MediaImageLoad.loadBitmap` loads a picture by URL straight into an upright image of the wanted side, with an optional circle mask. It works without an open layer.
- `MediaVoiceRecorder` records a voice message to an m4a file (AAC, mono) with its length, a wave level 0 to 1 while recording, the audio focus for recording and a configurable longest length. A refused microphone permission, a busy microphone and an empty recording are `MediaException.Voice` (Swift `MediaError.Voice`) with the same text on both platforms.
- Core `image_decode_plan`, `voice_level`, `voice_limit_ms` and `voice_refusal_text`.

## 3.11.1 - 2026-10-26

### Added
- Trace line `stage play dropped: <key> <reason>` when a play right is asked for and no play reaches the slot.

### Fixed
- Tearing down an on-screen probe no longer writes view state during the teardown (iOS exclusive-access crash on leaving a screen). `MediaImage` releases its place claim and `MediaVideo` re-declares its place on a source change without that write.

## 3.11.0 - 2026-10-26

### Removed
- Swift `localFileUrl(path:)`, `remoteUrl(text:)` and Kotlin `localFileUrl`, `remoteUrl`. The core applies one rule (`normalizeItem`) when it takes a declaration in.

### Changed
- `MediaSourceValue` initializers and factories keep the declared text and call nothing native. `MediaSourceValue.url`, `cover` and `blurHash` are the text as declared.
- A text that is no source url (`remote` with another scheme, `file` with a relative path, any other scheme) gives its place `MediaState.Failed` with `the source has no url`.
- Android image fetch and decode run outside the main thread by explicit Coil contexts.

## 3.10.1 - 2026-10-26

### Added
- Trace on Android: `adb shell setprop log.tag.MediaLayerTrace VERBOSE` writes core and player slot events to logcat under the tag `MediaLayerTrace`.

## 3.10.0 - 2026-10-26

### Removed
- Kotlin `MediaOutput` and the `output` parameter of `MediaVideo`.
- `VideoOutputCounters` entries `setVideoSurfaceView`, `clearVideoSurfaceView`, `surfaceCreate` and `liveSurfaces`.

### Changed
- Android video is always drawn through a `TextureView`.

## 3.9.3 - 2026-10-26

### Fixed
- Audio that finished playing starts from the beginning when shown again.

## 3.9.2 - 2026-10-26

### Fixed
- A seek issued before a place closes reaches the player.

## 3.9.1 - 2026-10-26

### Changed
- Nuke and Coil image decodes take memory from the shared budget while decoding.
- A refused memory budget during an image decode gives `MediaState.Failed` and `Media.failure`.
- An image with a side above 16384 is refused before decoding.

## 3.9.0 - 2026-10-26

### Added
- `MediaAudioSession.volumeRaises()` reports each raise of the device volume.

### Changed
- A refused memory budget reaches `Media.failure`.
- `prepareImage` refuses a photo with a side above 16384.
- Animation decodes share the memory budget of the library.

### Fixed
- Cancelling an Android rest frame decode stops it between frames.
- Cancelling an Android video tone read releases the reader at once.

## 3.8.0 - 2026-10-26

### Added
- `MediaLayer.open` takes defaults for `fetcher` and `settings`.
- `.mediaLayer(opening:)` and `MediaLayerProvider(Deferred)` connect the layer without readiness code.
- `MediaPlace`, `MediaWindow` and `PlaybackSuspension` objects own input through a bounded queue.
- `MediaPlaces.seek(key:fraction:)` and `MediaLayer.seek(key, fraction)` seek without a mounted view.
- `MediaAudio(onEnd:)` rewinds ended audio and reports the end once.
- `MediaState.isFailed`, `isSounding` and `isBuffering` answer common state questions.
- `MediaAudioSession.readVolume()` reads the Android volume.
- `MediaLayer.imageSize(source)` reads the image size from the file header.
- `MediaImage(_:preview:)` shows a local preview while the remote image loads.
- Windows report readiness, follow visible rows and support a reversed list.
- `resetUserData()` clears all data and the saved limits in one call.
- `MediaImage` reads `file://` sources through the core within the limit.
- `WithMediaTone` and `rememberMediaTone` observe the tone of a source.
- `View.mediaPlaybackSuspended` and `SuspendMediaPlayback` suspend playback while a view is mounted.
- `MediaWholeHold`, `mediaWholeDeclared` and `DeclareMediaWhole` declare sources to load whole.
- `MediaSourceValue` builds file, remote and bundled sources and cleans their url.
- Android `android.resource://` and `asset:///` sources load as local files.
- `MediaLayerTesting` and `:testing` give `TestFetcher` and `PlayerSnapshot` for app tests.
- `ImageAnimation` plays GIF and WebP with repeats, rest frame and reduced motion.
- `MediaLayer.mediaInfo(source)` reads duration and frame size of a local file.
- `prepareVideo` and `prepareImage` prepare media for sending.
- `MediaImageLoad.load` loads an image without opening a layer.
- `MediaError.Timeout` and `MediaException.Timeout` report a load that timed out.

### Changed
- `MediaImage` takes an `animation` argument.
- `MediaAudio` takes `playing` and `onEnd`, `AudioControl.seek` takes a fraction.
- Slot commands of `AVQueuePlayerSlot` and `ExoPlayerSlot` are private.
- `PlayerSlotFactory.create` is async in Swift and suspend in Kotlin and returns `CreatedPlayer`.
- `Media.setMuted(_:)` throws, and `Media.failure` reports the last open or input failure.
- Overloaded input reports `MediaError.Overloaded`, and closed objects report `MediaError.Closed`.
- `ImageWork.cancel` takes no key.
- Ended audio rewinds to the start and pauses.
- Android open prepares its resources off the calling thread.
- Closing the layer cancels declared loads and releases response bodies once.
- Mute on simulators and emulators comes from the library.

### Fixed
- Abandoned image decodes and queued waiters cancel at once.
- BlurHash decode memory stays within the response limit.
- A stale seek cannot cancel a newer seek.
- Slot commands run one at a time and ignore events of older loads.
- A delayed recovery action cannot apply to a newer load.
- A changed `playing` input reaches a mounted `MediaAudio`.
- The elected play intent applies when a source materializes.
- Open resources release when the open is cancelled or fails.
- Unfinished disk writes open off the polling thread.

### Removed
- The `silent` parameter of `MediaLayer.open` in Swift and Kotlin.
- The `key` field of `ImageWork.Cancel`.
- `attachView` and `detachView` of `MediaPlaces`, replaced by place objects.
- `replaceImageWindow`, replaced by `mediaImageWindow` and `MediaImageWindow`.

## 3.7.0 - 2026-10-23

### Added
- `MediaLayer.suspendPlayback(allowGroups:)` pauses every player until its `PlaybackSuspension` is released.
- `MediaVideo` and `MediaAudio` take a `group` that a suspension can allow to keep playing.
- Losing the audio output, such as unplugged headphones, turns the global mute on.

### Changed
- `MediaPlaces.setPlaying` and `MediaLayer.setPlaying` take `groups` with the groups of the playing views.
- `SystemSignalNames` takes an `outputLost` notification name.
- A changed `group` of a mounted `MediaVideo` or `MediaAudio` reaches the player choice.

## 3.6.2 - 2026-10-23

### Fixed
- Android first launch with another JNA library in the process cannot deadlock.

## 3.6.1 - 2026-10-23

### Fixed
- Audio seek before load keeps the fraction until the duration is known.

## 3.6.0 - 2026-10-23

### Changed
- Kotlin library uses Coil 3 and takes `coil3.ImageLoader` in `MediaLayerProvider` and `MediaLayer.image`.
- Kotlin `MediaImage` draws through `AsyncImage` without a painter per tile in composition.

## 3.5.0 - 2026-10-23

### Changed
- Swift `MediaImagePipeline.decodeSize` takes the `places` of the layer.

### Fixed
- `MediaImage` makes at most one core call per image on the main thread.

## 3.4.1 - 2026-10-23

### Fixed
- `MediaImage` composition takes image counters once per layer instead of once per image.

## 3.4.0 - 2026-10-23

### Changed
- `MediaLayer.seek` takes a fraction from 0 to 1 instead of milliseconds.
- Swift `AudioPlayback.seek(to:)` and Kotlin `AudioControl.seek(fraction)` seek by fraction.

### Fixed
- Audio seek works before the audio loads and applies when the slot is ready.

## 3.3.2 - 2026-10-22

### Fixed
- iOS xcframework build packs the static library in every slice.

## 3.3.1 - 2026-10-22

### Fixed
- Audio progress, seek and ended work when the host view has an empty or off-screen frame.

## 3.3.0 - 2026-10-22

### Changed
- Kotlin library requires Android 13 (API 33) or newer.
- Kotlin `MediaOutput.Window` draws video on a `SurfaceView` on every supported Android version.

### Fixed
- Android player switches video output on its own thread, not the main thread.

## 3.2.0 - 2026-10-22

### Changed
- Swift `MediaLayer.open` is `async throws` and builds the core off the calling thread.
- Kotlin `MediaLayer.open` returns `Deferred<MediaLayer>` and builds the core off the main thread.
- Kotlin `MediaLayerProvider` takes `Deferred<MediaLayer>` and waits for the open to finish.

## 3.1.0 - 2026-10-22

### Added
- `MediaLayer.close()` on iOS releases system signals, slots, images and the core once.

### Changed
- `limits()` and `stats()` throw `MediaError` when the library failed to assemble.
- Saved limits are read once while the library assembles and apply to every part.
- Prefetch, whole downloads and tone work wait in the background until the app returns.
- Slot commands of a replaced source are dropped before they reach the player.
- Audio stops only on the ended state reported by the core.
- Image exports do not change the state of a place.
- Video covers decode at their placement size on iOS and Android.
- Image cancellation statistics are one total instead of a map of URLs.
- The disk cache verifies each 256 KiB segment once before serving it.
- Cached responses are read in batches, with one buffer per batch.
- Held proxy requests wake on playhead events instead of polling the player.

### Fixed
- Resumed downloads reject a response with another `Content-Range` offset or length.
- A slow rejected response stops reading when its last consumer leaves.
- Connection permits return only after the platform closes the response.
- Header registry and playlist maps evict their oldest entries instead of clearing all.
- Declarations made before assembly keep the last request per key.
- A slot sample runs outside the stage lock.
- Native item states without an owning view leave no place behind.

## 3.0.0 - 2026-10-22

### Added
- `LoadedSlots.of(media)` returns the playback slots of one opened media object on iOS.

### Changed
- `MemoryReleaser` implementations provide `shutdown()`, called once when the media object closes.
- `mediaImageLoader` takes `ImageCounters` as a value instead of a stats lambda.
- On iOS, playback slots, view hosts and the image loader belong to one media object.
- On iOS, offscreen images and videos load and attach only inside the viewport.
- Animated GIFs on iOS decode as a stream and keep a bounded window of frames.
- GIF frames on iOS follow the delay of each frame.
- Image memory cache hits do not count as image requests.
- The disk cache index has a new format, existing caches are discarded on open.
- Cache writes flush once per commit, and the index saves only changed records.
- Saved disk limits apply before the first startup eviction.
- `purge(url)` cancels prefetch and tone work of that source only.

### Fixed
- Closing the media object on Android releases players, image memory, callbacks and the UI binding.
- On Android, a player state change wakes only the subscribers of that key.
- Readahead set before the Android player exists applies after the player loads.
- A failed cache file deletion keeps the entry and its disk accounting.
- Eviction keeps a newer version that replaced the selected entry.
- `purge` cancels prefetch before deleting files, so late writes do not recreate entries.

### Removed
- `MediaLayer.release()` on Android, use `close()`.
- `SlotPlayers.keyed` on Android, use `player(key)`.
- `LoadedSlots.shared` on iOS, use `LoadedSlots.of(media)`.

## 2.1.0 - 2026-10-21

### Added
- `media.imageStats().snapshot()` reports image requests, decoded pixels, placement pixels and cache hits.
- `MediaVideo` takes `output = MediaOutput.Window` for an experimental SurfaceView output on Android.
- `MediaVideo` takes `cornerRadius` and `alpha` parameters.
- Android traces show image requests, image decodes and video output attach calls.
- Video output counters report attach, detach and texture creation work.

### Changed
- Image decode size follows the placement size and treats `size` as an upper limit.
- `fetchImage` returns the bytes together with their source, cache or network.
- Playing slot counts include slots that preroll the next video.

## 2.0.0 - 2026-10-21

### Added
- `cache { free_space 25.percent }` limits the disk budget to a share of free space.
- `Limits.freeSpaceFraction` changes that share at runtime.
- `MediaLayer.open` on Android needs no fetcher or call factory.
- `MediaLayer.open` writes Android trace sections for startup measurements.
- `build_ios.sh --device` and `build_kotlin.sh --release` build only the release targets.

### Changed
- `MediaImage` has one rendering path and composes once in lists.
- `subscribeTone` takes a media item instead of a URL.
- Tone is prepared only on request, from a decode of at most 64 pixels.
- `imageMemoryBytes` is optional and defaults to a quarter of the Android memory class.
- `OpenConfig` takes `availableBytes` for the free disk space.
- Android fetching and playback share one HTTP client.
- ExoPlayer tries another decoder when the first one fails.
- Origin requests always ask for identity encoding.
- Playback progress is read once per key and only for active views.
- Playlists are parsed once and reused for playback and prefetch.
- Cache reads reuse the checked file for a response.
- Release builds use one code generation unit and narrower Android keep rules.

### Fixed
- HLS playlists arrive unpacked when the origin compresses them.
- Loaded animations stop outside the viewport.
- A whole download keeps separate entries for each header namespace.
- A stalled whole download times out when no bytes arrive.
- Android release builds include `core-ktx`.

### Removed
- `MediaWeight` and the `weight` parameter of `MediaImage`.
- `DEFAULT_IMAGE_MEMORY_BYTES`.

## 1.0.0 - 2026-10-21

### Added
- Media files (`.mdl`) declare video, image and audio sources and generate typed Swift and Kotlin.
- `MediaVideo`, `MediaImage` and `MediaAudio` declare a place and take a free player from the pool.
- A local caching proxy and one disk cache serve images, video and audio.
- Cache strategies `window`, `whole` and `on_demand` are chosen per source.
- Eviction strategies `lru`, `fifo`, `lfu`, `largest_first` and `expire_after` are chosen per kind.
- Limits change at runtime through `set_limits` and persist in the platform settings store.
- BlurHash, cover and first-frame placeholders are drawn before the media is ready.
- `tone` reads the average, dominant and band colors of a decoded image or video frame.
- `MediaAudioFocus` sets the iOS audio session and Android audio focus policy.
- `MediaLayer.cacheKey(url, headers)` gives native image loaders the identity of the disk cache.
- One-shot image export, `MediaOutput.Window`, `MediaWeight.Tile`, playback position and playback rate.
- Memory pressure levels recycle players and shrink caches.
- Every cache entry carries its length and a SHA-256 prefix checked on first read.

### Changed
- `open(spec)` applies `forward_headers` of the first video before the origin is used.
- `open(spec)` applies the eviction override of the first declaration of each kind.
- `MediaVideo` plays by default on both platforms.
- The generator rejects duplicate options, numbers that do not fit and colliding type names.
- The generator compile check requires `swiftc` and `kotlinc`.

### Fixed
- A `whole` download reports success only when the cache holds the full source.
- A source warmed by a window loads the remaining bytes when it is declared `whole`.
- `clear` and `purge` drop the old playlist plan of an HLS source.
- A declaration without headers does not reuse the cached answer of the previous user.
- A late memory-pressure result does not unload newly assigned sources.
- Cancelling an asynchronous call cancels its inner work and frees the response body.
- The last subscription to an unused place releases the place.
- A failed cache index write is retried.
- Lowering and raising the concurrency limit keeps the reserve for playback.
- Sampling a native player does not wait for its creation.
- Replacing the URL of a video or audio view releases the previous source.
- Progress polling stops in the background and off screen.
- Image caches keep separate entries for one URL with different headers.

### Removed
- `transport_headers`.

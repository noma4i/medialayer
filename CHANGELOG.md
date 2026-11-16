# Changelog

## Unreleased

### Added

- `Media.retry(source)` reloads a failed source without remounting its places.
- `VoiceState.recordedMs` publishes the elapsed recording time.
- `MediaVoiceRecorder` accepts `minTakeMs` and refuses shorter takes.
- Each place publishes `PlaybackState` with playing, wish, pause reason and play refusal.
- `MediaVideo(onPlaybackChange:)` reports each change of the core `PlaybackState`.
- Swift `AudioPlayback` gains `play()` and `pause()` like Kotlin `AudioControl`.
- `readRoute(item)` returns the local proxy route without starting playback.
- `MediaVoiceRecorder.state` and `states` show the take, its phase and its outcome.
- `CameraState` gains `take` and `outcome`, and `CameraPhase` gains `closed`.
- `MediaCallAudio.route` shows the applied call output or its failure.
- `Media.failures` streams every native failure with its source, domain and code.
- The screen stays on while a `MediaVideo` or `MediaAudio` place plays (`PlaybackState.playing`) and is released when the last playing place of the window pauses, ends or leaves.
- One core rule owns the audio session, audio focus, recording and calls on both platforms.
- `callAudio(kind)` takes a CallKit, Telecom or unmanaged call with call states.
- `microphoneAvailability` names who holds the microphone, including calls of other apps.
- `VoiceRefusal` and `CameraRefusal` gain `interrupted` and `microphoneSilent`.
- A video take with sound starts only while the microphone sound flows.
- `MediaLayer.callAudio` holds a call and its audio session, with speaker and CallKit control.
- `CameraRefusal.microphoneBusy` names a camera refusal caused by a busy microphone.
- `MediaAudioSession.holdRecording()` gives a recording hold that its owner releases.
- The trace writes audio session steps, recording holders and camera audio tracks.

### Changed

- `playUrl` returns a library `MediaPlayer` with its own playback state.
- A play during a call or recording is refused and shows its refusal.
- After the library forgets a wish, one Play plays again.
- `MediaError.Voice` carries the `VoiceRefusal` reason.
- Kotlin `MediaCallKind.Telecom` takes the call endpoint control of the app.
- Speaker changes of CallKit and Telecom calls route without a new audio session.
- A lost or failed call session shows the call as interrupted until it returns.
- `microphoneAvailability` follows permission changes, other apps and refused sessions.
- Closing the layer ends cameras, recorders, calls, volume and availability streams.
- The audio session changes only after the recorder and players stopped.
- Swift `statsStream` throws its failure: read it with `for try await`.
- Kotlin `readVolume` is a suspend function and reads the volume off the main thread.
- `FailureListener` gains `closed`, and failure streams end when the layer closes.
- `Media.muted` changes only after the core accepted the mute.
- `clear` returns the error of the cache instead of hiding it.
- `PlaceInput` gains `playGeneration`: Kotlin callers pass its fields by name.
- A recording pauses playback, and its end resumes nothing.
- A lost headphone or Bluetooth output pauses playback and keeps `Media.muted` unchanged.
- An interruption resumes playback only when the system allows it.
- Android players share one audio focus request held by the library.
- The iOS recording session keeps headphones and Bluetooth as the output.
- `callAudio(kind)` throws `Closed` after the layer closes, and Swift `callAudio` is `async throws`.
- Kotlin `MediaVoiceRecorder.start` is a suspend function.
- Audio session, focus and speaker errors reach `Media.failure` with their system domain and code.
- The audio session follows the recording and playback demand after any system event.
- Internal test hooks do no work in release builds.
- The outcome of a recording is given once; a second stop is refused until the next start.
- A recording is turned upright while the camera is stopping, and a cancel then stops the conversion.
- A recorded video comes out upright in its pixels, with no rotation tag.

### Fixed

- A place that starts playing while a camera preview or recording holds the session plays at once.
- The screen stays on while a place plays with sound, a voice take, a camera preview or a call runs.
- `isPlaying` shows paused after the library pauses a place.
- A muted or buffering place pauses on a call, recording or interruption.
- A take finished by the duration limit reaches the app without a stop.
- A take lost by a media services reset shows its loss.
- A failed recorder ends the take with its native failure and deletes the file.
- A new call after a permanent focus loss takes its audio session.
- Kotlin camera and volume calls run off the main thread.
- Kotlin frame observers all receive frames.
- A file that cannot be deleted is reported instead of ignored.
- A call or recording changes the audio session only after playback has paused.
- Sound returns after a recording, a call or an interruption without an end.
- The first video note after a cold start records its sound or fails.
- An own Android call is not counted as an external call.
- A refused or delayed audio request recovers on a new sound, recording, call or volume reader.
- A change of audio demand during a pending system request is not lost.
- A switch from Bluetooth A2DP to HFP does not count as a lost output.
- The camera releases its microphone and memory holds when its start is cancelled.
- The end of an own call clears its communication mode on Android.
- Only a lost headphone, Bluetooth or other private output turns the global mute on.
- Sound comes back after a recording, an interruption or a media services reset.
- An Android audio focus taken by the system is requested again.
- A second voice recording start during the first is refused.
- The iOS camera preview keeps its frames when a recording starts or stops.
- A lens change asked while the camera stops or takes a photo is applied when that ends.
- A camera and a voice recorder can hold the recording audio together.
- An iOS voice recording stopped right after its limit gives its file and ends once.
- An Android layer released twice reports no receiver error.
- A peeking neighbour page stays paused while a call hold is active.
- A call hold never hands playback to another waiting video.
- Closing a sibling place after a call keeps its key paused until a new declaration.
- A cancelled or cut block download keeps its bytes and resumes from where it stopped.
- A pending playback retry survives a stall or a pause of the same video.
- A source evicted from the cache is prefetched again when it re-enters the window.
- An Android video never shows its old cover after its cover changes under the same url.
- A one-shot video place ends once even when a window loaded the clip first.
- A fixed crop frame turned right and then left returns to the frame it had.
- Playback after a voice recording starts in the playback audio session on iOS.
- An Android image is drawn only in the frame where its content gets the ready state.
- An Android video starts no cover request while it is off screen.

### Removed

- `callHold()`, replaced by external call detection and `callAudio(kind)`.
- `callAudio(managedBySystem:)`, `liveChanges` and `live`, replaced by `callAudio(kind)` and `states`.
- `MediaAudioSession.holdRecording()`, recorders hold the microphone themselves.
- `SystemSignalNames.outputLost`.
- `MediaAudioSession.activateRecording()` and `deactivateRecording()`, replaced by `holdRecording()`.

## 3.33.1 - 2026-11-14

### Fixed
- A paused audio keeps its playhead position while the player reloads.
- The playhead ignores a late player report that is up to a second behind.

## 3.33.0 - 2026-11-14

### Added
- Audio exposes a playhead that the view draws at the display frame rate.

## 3.32.0 - 2026-11-13

### Added
- A local video without a cover shows its first frame until the video renders.

### Changed
- Importing a picked file on Android copies it in the kernel when the provider gives a descriptor.

## 3.31.0 - 2026-11-13

### Changed
- Background unloads every player slot, including the playing one, and drops the image memory cache.

### Fixed
- An origin that answers 200 to a range request is served from the whole body.

## 3.30.8 - 2026-11-13

### Fixed
- The camera preview no longer blocks the main thread while the camera starts or stops.

## 3.30.7 - 2026-11-12

### Fixed
- `prepareVideo` names the decoder maximum when the device cannot decode the source.

## 3.30.6 - 2026-11-12

### Fixed
- `MediaVideo` reports progress 1.0 before `onEnd`, also when `onEnd` removes the view.

## 3.30.5 - 2026-11-12

### Fixed
- Playback progress reaches 1.0 when a video ends, also when the video track is shorter than the item.

## 3.30.4 - 2026-11-12

### Fixed
- Android `MediaVideo` keeps its video frame size when the player leaves the view, so the outgoing feed video is not resized and does not flash black at swipe start.

## 3.30.3 - 2026-11-12

### Fixed
- Android `MediaVideo` with `ContentScale.Crop` fills its box like the cover does (same scale, same centre), no letterbox around a landscape video in a portrait box. `Fit` letterboxes both.
- Android `MediaVideo` shows its cover from the first composition until the first rendered frame, no black TextureView while the core has not answered the placeholder yet.

## 3.30.2 - 2026-11-12

### Changed
- Android fills the media3 decoder lists for H.264, H.265 and AAC once per process on a background thread when the library opens, so the first feed video no longer pays that lookup on the playback thread.

### Fixed
- Android `MediaVideo` keeps its last video frame on screen after the player leaves the view, until the view changes source. An outgoing feed video no longer goes black while it slides out.

## 3.30.1 - 2026-11-12

### Changed
- Android `MediaImage` reads one process-wide reduce-motion observer instead of registering a `ContentObserver` per image.

## 3.30.0 - 2026-11-11

### Added
- Swift `.mediaShape(_:)` with `MediaShape` (`.rectangle`, `.circle`, `.rounded(_:)`) rounds the `CALayer` that owns the contents of `MediaImage` and `MediaVideo`.
- Swift `.mediaGrayscale(_:)` draws `MediaImage`, still and animated, in grayscale: each frame is converted once, off the main thread for animations.

### Changed
- Swift visibility runs one pass per display frame for all observed views, and a frame without a change runs no pass.
- Swift animated images set `CALayer.contents` on a display-link tick with frames prepared off the main thread, no `UIImageView.image` write per frame.

### Migration
- iOS app: replace `.grayscale(1)` over `ChatGif` (`VideoNoteGifPreviewComponent`) with `.mediaGrayscale()` on the `MediaImage`.
- iOS app: replace `.clipShape(Circle())` over the video note with `.mediaShape(.circle)` on the `MediaVideo` (and on the `MediaImage` of its preview).
- Android: no equivalent in this release, iOS only. The Kotlin `MediaImage` and `MediaVideo` keep their current drawing, the Android app keeps its own clip and colour filter until a Kotlin option is asked for.
- `FrameImageView` is an internal type and is gone. App code does not use it, tests of the library use `FrameLayerView`.

## 3.29.1 - 2026-11-11

### Fixed
- Cancelling `prepareVideo` no longer crashes while a frame is being encoded.

## 3.29.0 - 2026-11-11

### Added
- `failureStream()` on Swift and `failureFlow()` on Kotlin report every load and store failure.
- Publish `com.noma4i:medialayer-testing` with `TestFetcher` for Android tests.
- Kotlin `MediaImage` plays animated images at a given playback rate.

### Changed
- prepareImage and encodeImage write the encoded photo to the file as it is encoded.
- `MediaImage` content gets the ready state in the frame the image is drawn.
- Swift `MediaImage` no longer cross-fades the placeholder into a cold image.
- The cache budget is the configured budget, a full phone no longer blanks images and video.
- Image loads fall back to a direct fetch when the disk cache refuses to store them.

### Fixed
- A video window without sources no longer takes the stage from the active window.

### Removed
- The `free_space` cache option, `Limits.freeSpaceFraction` and `OpenConfig.availableBytes`.

## 3.28.0 - 2026-11-06

### Added
- `callHold()` stops playback and recording while a call runs and restores nothing on release.

## 3.27.1 - 2026-11-02

### Fixed
- iOS `prepareVideo` returns a clean video untouched when its container tag is escaped as a Latin byte (such as `%A9too`), instead of copying it.
- An image window becomes ready when a source of its page failed before the window started following the page.

## 3.27.0 - 2026-10-31

### Added
- `importMedia(source)` copies picked media into a caller-owned file without re-encoding.

## 3.26.3 - 2026-10-29

### Added
- Android releases provide `medialayer` and `medialayer-ui` Maven modules.
- Releases provide a root SwiftPM package manifest for the published binary.

## 3.26.2 - 2026-10-27

### Fixed
- Window readiness stays true when the active page moves to an already cached page.

## 3.26.1 - 2026-10-27

### Fixed
- Image requests prepare cache keys and decode sizes outside the main thread.

## 3.26.0 - 2026-10-27

### Added
- `MediaSourceValue.raw`, `asset` and `bundled` take an image: `MediaImage` opens an Android drawable or asset on `Dispatchers.IO` and decodes it off the main thread with the usual memory admission and memory cache.

### Removed
- `bundledSourceError` (`bundled_source_error`) and the programming error of a bundled image source.

## 3.25.0 - 2026-10-27

### Added
- `MediaImageEncode.encode` encodes image bytes as JPEG or PNG without an open MediaLayer, for a notification extension or a push service, and refuses a raster above `limitBytes` with `MediaError.Overloaded`.

## 3.24.0 - 2026-10-26

### Changed
- A video cover stays until the first frame renders, and returns show no cover.

### Removed
- `CacheState` and the cache argument of `placeholderFor`.

## 3.23.1 - 2026-10-26

### Fixed
- Swift `MediaVideo` keeps its first-frame flag for the same source when the page leaves the screen and returns, as Compose does. `onFirstFrame` no longer fires again on every return.

### Added
- The trace prints `view state <key tail> <state>` and `view placeholder <key tail> blur|cover|none` for `MediaVideo` on both platforms, so a cover that is shown or removed during a page change can be read next to `view onScreen`.

## 3.23.0 - 2026-10-26

### Added
- `MediaCropSnapshot` carries `crop` and `rotation`, the area `confirm()` would give now, and `snapshot()` leaves the crop open.

### Fixed
- A gesture with a pan and a pinch calls `onPositionChange` twice, the pan report and then the pinch report, as in V2.

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

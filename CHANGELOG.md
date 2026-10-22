# Changelog

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

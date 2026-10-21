# Changelog

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

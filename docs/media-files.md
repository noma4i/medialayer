# Media files

You describe media once in a `.mdl` file. The generator produces typed sources for Swift and Kotlin and the settings for the core. App code never builds URLs, cache keys or prefetch windows. This page lists every option, its default and what it does.

```ruby
cache do
  budget 4.gib, min: 64.mib
  shrink_to 90.percent
  evict :lru
  memory 256.mib
  timeout 30.seconds
  max_concurrent 2
  dedupe window: 60.seconds, entries: 500
  prefetch_budget 64.mib
  namespace headers: [:authorization]
end

video :moment_video do
  cover :coverUrl
  prefetch from: 1, to: 3, chunks: [2, 1]
  playback chunks_ahead: 2
  pool slots: 3, reach: 1, hold_off: 1.second, trim_delay: 30.seconds, low_ram_slots: 2, low_ram_reach: 0
  forward_headers [:authorization, :cookie, :user_agent]
  evict :lfu
  strategy :whole
end

image :avatar do
  blurhash :blurHash
  size 288, 288
  prefetch window: 1...3, behind: 2
end

audio :voice_note do
  evict :fifo
end
```

Durations are written `30.seconds`, `5.minutes`, `2.days`. Sizes are written `64.mib`, `4.gib`. Percentages are written `90.percent`.

## `cache`: one block for the whole app

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `budget` | 4 GiB | Disk size at which eviction starts. When the cache reaches it, entries are removed until it shrinks to `shrink_to` |
| `min` | 64 MiB | Smallest budget the app or the user may set. A lower value is rejected |
| `shrink_to` | 90% | Share of the budget the cache shrinks to after it reaches the budget. In (0, 100] |
| `evict` | `:lru` | Which entries go first. See the strategies below |
| `memory` | 256 MiB | One memory limit for players, proxy buffers and image memory. When it is exceeded, the library frees the least needed slots |
| `timeout` | 30 seconds | Longest wait for a network response before a load fails |
| `max_concurrent` | 2 | Prefetch loads that run at once. Loading what is playing never waits for them |
| `dedupe window:` | 60 seconds | Time during which a repeated prefetch of the same source is skipped |
| `dedupe entries:` | 500 | Most sources remembered for that skip. The oldest are dropped |
| `prefetch_budget` | 64 MiB | Hard limit for the bytes of prefetched stream chunks that were never played. 1 to 512 MiB. When a new chunk would go over it, the chunks of the farthest items are removed first. A chunk of the farthest item is not fetched at all, and that item settles as over budget |
| `namespace headers:` | none | Request headers that become part of the cache key. By default the key is the URL alone. Set it when the same URL returns different bytes for different users. The key then is the URL plus a hash of the listed header values; a request without those headers keeps the plain URL key. `purge(url)` removes every variant |

`memory` counts players, proxy buffers and the decode of an image in a view for the time of the decode. The decoded image kept in the Nuke or Coil cache has its own limit, `imageMemoryBytes` of `open`.

The cache key is the URL. The cache lives on disk; image memory caches stay with Nuke and Coil. An entry being read or written is never evicted. The budget is the configured `budget`; free disk space never changes it. If new data does not fit after eviction, or the disk refuses the write, the bytes are still delivered: an image is shown from memory and a video plays from the network. The store failure is reported on the failure stream (`docs/failures.md`).

### Eviction strategies

| Strategy | Removes first |
| -------- | ------------- |
| `:lru` | The entry used longest ago |
| `:fifo` | The entry stored first |
| `:lfu` | The entry used the fewest times |
| `:largest_first` | The biggest entry |
| `expire_after 7.days` | Entries older than the given age, then the oldest. Written `evict expire_after 7.days` |

`evict` inside `video`, `image` or `audio` overrides the strategy for that kind only.

## `video`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `cover` | none | Name of the field of your own model that holds the cover image URL (the symbol is only a name for your data). The generated type gets an optional `cover` parameter. If given, the cover shows until the video is ready. If not, the first frame is cut from the first chunk and shown instead |
| `prefetch from:, to:` | 1, 3 | Pages ahead of the active page that are loaded in advance. Both ends are included: `from: 1, to: 3` loads the pages 1 to 3 ahead |
| `prefetch chunks:` | `[2, 1]` | How many chunks (HLS segments) of each prefetched stream are loaded, by distance from the active page. The page 1 ahead gets 2 chunks, every page farther 1. The list has 1 to 8 entries, each 1 or 2 (the owner cap: prefetch is never more than 2 chunks), and never grows with the distance. A wrong list is an error, it is never cut. The init segment is loaded with the first chunk and is not counted. Moving the active page loads only the missing chunks of an item that came closer, and cancels the deeper requests of an item that went away. Loaded chunks stay in the cache until its budget removes them |
| `playback chunks_ahead:` | 2 | How many chunks (HLS segments) ahead of the playing position the library loads from the network, 1 to 8. The local proxy does not fetch a chunk beyond the position plus this number until the player gets closer, so a player that asks for the whole clip cannot pull it. A chunk that is already in the cache is never held, a looping clip replays from the cache. A stall of the playing item adds 1 chunk for that item, 8 at most. The player buffer hint is derived from it. A wrong value is an error that names the key |
| `pool slots:` | 3 | Number of native players in the pool. A low-memory device gets `low_ram_slots` instead |
| `pool reach:` | 1 | Pages kept ready on each side of the active page |
| `pool hold_off:` | 1 second | Time before a slot released under memory pressure may load again |
| `pool trim_delay:` | 30 seconds | Time before an idle slot drops from hot to warm to cold |
| `pool low_ram_slots:` | 2 | Number of players on a low-memory device. At least 2 |
| `pool low_ram_reach:` | 0 | Pages kept ready on each side of the active page on a low-memory device |
| `forward_headers` | Authorization, Cookie, User-Agent, Connection, Accept, Accept-Language, Range | Request headers passed to the origin server |
| `evict` | the `cache` strategy | Override of the eviction strategy for videos |
| `strategy` | `:window` | How the library caches and preloads this video: `:window`, `:whole` or `:on_demand`. See [cache strategies](cache-strategies.md) |

Opening a spec uses `forward_headers` from its first video for all origin requests, matching the selection of pool settings and limits.
An empty list forwards no source headers. Without a video declaration, the default list applies.
The proxy creates Range itself and never forwards a client's Range unchanged.
The proxy always sends `Accept-Encoding: identity` to the origin, even when `forward_headers` includes `Accept-Encoding`.

The playing video always comes first. Until the playing video shows its first frame, no prefetch request starts and every running one is stopped. When the active page changes, the requests of pages that left the window are cancelled at once. The player has its own reserved connection, so prefetch never makes it wait, and when the player asks for a segment that prefetch is already downloading, it joins that download. After the first frame, prefetch continues and loads only the chunks that are still missing.

## `image`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `blurhash` | none | Name of the field of your own model that holds the BlurHash string. The generated type gets an optional `blurHash` parameter. If given, a blurred preview shows until the image loads. If not, the place stays empty |
| `size` | none | Pixel size images are decoded at. It always wins. Without it, the image is decoded at the size of its view in pixels: points times the screen scale, rounded up to a multiple of 8. Smaller means less memory |
| `prefetch window:` | `0...9` for a window without a block | Range of pages, counted from the active page, whose images are loaded in advance. `1...3` means pages 1 and 2 ahead (the end is not included, as in Ruby). Applies to the image window that has the name of the block |
| `prefetch behind:` | 0 | Pages behind the active page whose images are loaded in advance |
| `evict` | the `cache` strategy | Override of the eviction strategy for images |
| `strategy` | `:window` | `:window`, `:whole` or `:on_demand`. See [cache strategies](cache-strategies.md) |

### Image windows

A grid, a chat or a feed keeps one image window per list. The window has a name. A page is `rowsPerPage` neighbouring rows, and every row can hold several images. `mediaImageWindow` (SwiftUI) and `MediaImageWindow` (Compose) take the name, your rows, the sources of one row and the active page. The window loads the images of the pages around the active page into the disk cache, nearest pages first. A window is independent of other windows and of the video window. An image that two windows want loads once. An empty list of rows cancels the window.

A window named like an `image` block takes the range of that block (`prefetch window:` and `prefetch behind:`). Any other name takes the default: 2 pages behind, pages 0 to 8 ahead. A `file://` source is never loaded.

#### Swift

```swift
ChatScreen()
  .mediaImageWindow(name: "avatar", of: feed, rows: \.rows, sources: { [Avatar(url: $0.avatarUrl).item] }, active: rowIndex)
```

#### Kotlin

```kotlin
MediaImageWindow(name = "avatar", data = { feed.rows }, sources = { listOf(Avatar(url = it.avatarUrl).toItem()) }, active = rowIndex)
```

#### Visible rows

A list does not have to compute `active`. Without `active`, the window follows the rows on screen: its active page is the page of the first visible row. In SwiftUI each row calls `mediaWindowRow(name, index:)` and the window above it with the same name receives the rows that appear and disappear. In Compose the window reads the visible items of a `LazyListState` or a `LazyGridState`; the index of an item is the index of its row. The window does not move while no row is visible.

A list that shows its newest row first sets `reversed`. The pages then count from the last row, so the pages ahead are the rows above the screen, and the active page is the page of the last visible row. An empty row stays in its page. Set `rowsPerPage` to 1 for one page per row. A row holds zero or more images.

#### Swift

```swift
List(rows.indices, id: \.self) { index in Bubble(rows[index]).mediaWindowRow("chat", index: index) }
  .mediaImageWindow(name: "chat", of: thread, rows: \.rows, sources: { $0.photos }, reversed: true)
```

#### Kotlin

```kotlin
MediaImageWindow(name = "chat", data = { thread.rows }, sources = { it.photos }, state = listState, reversed = true)
```

#### Ready

The window tells when its active page is prepared. A video window is ready when the active source has shown its first frame or is already in the cache. An image window is ready when every image of the active page is in the cache or has failed. A change of the active page, or of the sources on it, makes the window not ready until the next event. The window needs no timer and no polling: show the list when `ready` turns true. A list that only grows keeps the readiness it has while the active page keeps the same sources.

#### Swift

```swift
struct FeedPage: View {
  let feed: Feed
  let page: Int
  @State private var windowReady = false

  var body: some View {
    ChatScreen()
      .mediaWindow(of: feed, rows: \.rows, source: { $0.video }, active: page, ready: $windowReady)
  }
}
```

#### Kotlin

```kotlin
val windowReady = MediaWindow(data = { feed.rows }, source = { it.video }, active = page)
```

### Image size and memory

`MediaImage` reads the size of its view and the screen scale and asks the decoder for exactly that many pixels, so a grid never decodes full-size photos. `contentMode` (Swift) and `contentScale` (Kotlin) choose between filling and fitting the view. The decoder keys memory by the core cache identity, decoded size and content mode. Selected namespace headers separate images, covers and animated frames in memory. An image that is already there shows in the first frame, without a placeholder. `imageMemoryBytes` sets an explicit budget. By default Android uses `min(128 MiB, 25% memoryClass)` and Swift uses 128 MiB. `clear()` and `clear(image)` also empty this memory.

## `audio`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `evict` | the `cache` strategy | Override of the eviction strategy for audio |
| `strategy` | `:window` | `:window`, `:whole` or `:on_demand`. See [cache strategies](cache-strategies.md) |

## What the generator makes

One type per block, for example `MomentVideo`, `Avatar` and `VoiceNote`. Every type has `url`. A video with `cover` and an image with `blurhash` also get that optional parameter, named `cover` and `blurHash`. The names of the parameters come from the option names, not from the symbols.

The parser rejects repeated option keys and keys attached to a top-level block.
Numeric conversions reject overflow before building the spec.
Image dimensions must fit Kotlin `Int`, from 1 through 2147483647.
Source names cannot collide with types referenced or declared by the generated Swift or Kotlin code.

### Checking generated consumers

Run Rust checks with `cargo test -p medialayer-gen`.
The full check generates sources from the DSL fixture in `gen/fixtures`, compiles them with `gen/fixtures/Consumer.swift` and `Consumer.kt` against the real library and runs the consumers as tests.
Each consumer compares the `MediaItem` and the size of every generated source with the values written in the fixture.
The Rust-only command does not prove compilation or values in either language.

| Check | Command | Run by |
| --- | --- | --- |
| Swift on iOS Simulator | `scripts/check_gen_consumers.sh swift [udid]` | `scripts/check_swift_test.sh` on an iOS Simulator with `-only-testing:MediaLayerTests/GeneratedSourcesTests`, started by the Rust test, which places the sources in the Swift package tests and removes them |
| Kotlin on brick | `scripts/check_gen_consumers.sh kotlin-run <command>` | The gradle-test step of the check runs through it: it clears and places the generated sources in the ignored Kotlin test sources, runs Gradle on brick and removes them |

A missing compiler, a generator error, a compile error or a wrong value fails the check.
The Kotlin sources are generated files and stay out of git.
The fixture covers all source kinds, explicit and default cache strategies and the maximum accepted image dimensions.

### Swift

```swift
let video = MomentVideo(url: moment.videoUrl, cover: moment.coverUrl)
let avatar = Avatar(url: user.avatarUrl, blurHash: user.blurHash)
let note = VoiceNote(url: message.audioUrl)
```

### Kotlin

```kotlin
val video = MomentVideo(url = moment.videoUrl, cover = moment.coverUrl)
val avatar = Avatar(url = user.avatarUrl, blurHash = user.blurHash)
val note = VoiceNote(url = message.audioUrl)
```

Every value in the file is a default. The generated `MediaSpecJSON` holds all of them; pass it to `open` as `spec` and the core takes the limits of the `cache` block and the first `video`, the pool settings of the first `video` and the namespace headers. The app or the user can change limits at run time and the new value is saved on the device. A saved value wins over the file at the next launch, and opening never writes to the saved settings. A spec that does not parse or does not validate throws `InvalidSpec`.

### Swift

```swift
let media = try await MediaLayer.open(
  spec: specJson,
  fetcher: URLSessionFetcher(),
  settings: UserDefaultsSettings()
)
```

### Kotlin

```kotlin
val media: Deferred<MediaLayer> = MediaLayer.open(
  context = context,
  spec = specJson,
  settings = SharedPreferencesSettings(context)
)
```

## Silence on simulators and emulators

The library keeps every player muted on the iOS Simulator and on an Android emulator. No place and no `Media.muted` can unmute them. `open` has no option for this and the app passes nothing. On a device the players follow the places and `Media.muted`.

## Options of `open`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `fetcher` | Swift: required, Kotlin: `null` | Core network. Android creates `OkHttpFetcher` with the client shared with the players |
| `settings` | none | Local storage for saved limits |
| `spec` | the defaults of the core | JSON from `medialayer-gen spec`. Gives the default limits, the pool settings and the namespace headers |
| `directory` (Swift) | `Caches/MediaLayer` | Folder of the disk cache. The cache folder of the app on Kotlin |
| `callFactory` (Kotlin) | `null` | Android creates one shared client for the default fetcher and the players. An explicit value replaces the players client |
| `audioFocus` | `mix` | What the players do to other apps' music: `exclusive`, `mix` or `duck`. See [audio focus](audio-focus.md) |
| `imageMemoryBytes` | Swift: 128 MiB, Kotlin: `null` | Android picks `min(128 MiB, 25% memoryClass)`. An explicit positive value replaces the automatic cache budget |

Android lets ExoPlayer pick the next available decoder after an init error. The default decoder order stays. A software decoder can use more CPU.

## Local files

A source with a `file://` URL, such as a clip in the app bundle or a temporary file, plays from its own path. The core does not fetch it, store it in the cache or prefetch it. It counts as cached, so it plays at once. A video with a `file://` URL and no cover shows its first frame until the first frame is rendered. A video that names a cover, and an image, show no placeholder. The same applies to the `loop`, `onFirstFrame` and `onEnd` options of the place. An image with a `file://` URL is read from its file by `fetchImage`: it is never copied to the disk cache or prefetched. The read goes through the core on both platforms and stays within the response limit (16 MiB): a larger file fails like any other load. One mounted `MediaVideo` or `MediaImage` with a `file://` URL loads, renders and plays without a declared window.

## Importing picked media

`MediaLayer.importMedia(uri)` on Kotlin and `MediaLayer.importMedia(url)` on Swift copy picked media without re-encoding.
Android uses the context passed to `open` and accepts content URIs.
Swift accepts readable file URLs, including security-scoped URLs.
The result is a new `PreparedMedia` with `isNew` true, stored in the app's temporary directory.
The caller owns this file and deletes it after use.
Pass its file URL to `MediaSourceValue.file` for `prepareImage`, `prepareVideo` or `MediaCropper`.

Copies read at most 64 KiB per chunk, outside the main thread.
Open, read and write errors throw `MediaException` on Kotlin or `MediaError` on Swift.
Cancellation closes streams and deletes the incomplete copy.
A finished copy remains until the caller deletes it.
The extension uses the display name first, then a known media MIME type, and stays empty when neither identifies it.

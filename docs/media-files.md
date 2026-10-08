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
  namespace headers: [:authorization]
end

video :moment_video do
  cover :coverUrl
  prefetch from: 2, to: 5, depth: 2.seconds
  pool slots: 3, reach: 1, hold_off: 1.second, trim_delay: 30.seconds, low_ram_slots: 2, low_ram_reach: 0
  forward_headers [:authorization, :cookie, :user_agent]
  evict :lfu
end

image :avatar do
  blurhash :blurHash
  size 288, 288
  prefetch window: 1...3
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
| `namespace headers:` | none | Request headers that become part of the cache key. By default the key is the URL alone. Set it when the same URL returns different bytes for different users. The key then is the URL plus a hash of the listed header values; a request without those headers keeps the plain URL key. `purge(url)` removes every variant |

UNBUILT: `memory` counts players and proxy buffers. Image decoding in Nuke and Coil is not counted yet.

The cache key is the URL. The cache lives on disk; image memory caches stay with Nuke and Coil. An entry being read or written is never evicted. If new data still does not fit after eviction, the load fails with `NotEnoughSpace`.

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
| `prefetch from:, to:` | 2, 5 | Pages ahead of the active page whose first chunk is loaded in advance. Both ends are included: `from: 2, to: 5` loads the pages 2, 3, 4 and 5 ahead |
| `prefetch depth:` | 2 seconds | How much of the start of each prefetched video is loaded |
| `pool slots:` | 3 | Number of native players in the pool. A low-memory device gets `low_ram_slots` instead |
| `pool reach:` | 1 | Pages kept ready on each side of the active page |
| `pool hold_off:` | 1 second | Time before a slot released under memory pressure may load again |
| `pool trim_delay:` | 30 seconds | Time before an idle slot drops from hot to warm to cold |
| `pool low_ram_slots:` | 2 | Number of players on a low-memory device. At least 2 |
| `pool low_ram_reach:` | 0 | Pages kept ready on each side of the active page on a low-memory device |
| `forward_headers` | Authorization, Cookie, User-Agent, Connection, Accept, Accept-Encoding, Accept-Language, Range | Request headers passed to the origin server |
| `evict` | the `cache` strategy | Override of the eviction strategy for videos |

## `image`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `blurhash` | none | Name of the field of your own model that holds the BlurHash string. The generated type gets an optional `blurHash` parameter. If given, a blurred preview shows until the image loads. If not, the place stays empty |
| `size` | none | Pixel size images are decoded at. Smaller means less memory |
| `prefetch window:` | none | Range of items ahead of the visible ones whose images are loaded in advance. `1...3` means items 1 and 2 ahead (the end is not included, as in Ruby) |
| `evict` | the `cache` strategy | Override of the eviction strategy for images |

## `audio`

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `evict` | the `cache` strategy | Override of the eviction strategy for audio |

## What the generator makes

One type per block, for example `MomentVideo`, `Avatar` and `VoiceNote`. Every type has `url`. A video with `cover` and an image with `blurhash` also get that optional parameter, named `cover` and `blurHash`. The names of the parameters come from the option names, not from the symbols.

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

Every value in the file is a default. The generated `MediaSpecJSON` holds all of them; hand the values to `OpenConfig` (`defaults`, `pool`, `namespaceHeaders`). The app or the user can change limits at run time and the new value is saved on the device.

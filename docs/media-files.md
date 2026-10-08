# Media files

You describe media once in a `.mdl` file. The generator produces typed sources for Swift and Kotlin and the settings for the core. App code never builds URLs, cache keys or prefetch windows.

```ruby
cache do
  budget 4.gib, min: 64.mib
  evict :lru
  shrink_to 90.percent
  memory 256.mib
end

video :moment_video do
  cover :coverUrl
  prefetch from: 2, to: 5, depth: 2.seconds, timeout: 30.seconds
  pool slots: 3, reach: 1
end

image :avatar do
  blurhash :blurHash
  size 288, 288
end

audio :voice_note do
  evict :fifo
end
```

| Block | Keys |
| ----- | ---- |
| `cache` | `budget`, `min`, `shrink_to`, `evict` (`:lru`, `:fifo`, `:lfu`, `:largest_first`, `expire_after 7.days`), `memory`, `namespace headers: [...]` |
| `video` | `cover` (optional), `prefetch`, `pool`, `forward_headers`, `evict` |
| `image` | `blurhash` (optional), `size`, `prefetch window: 1...3`, `evict` |
| `audio` | `evict` |

Every value is a default. Users and apps can change limits at run time ([limits and eviction](limits-and-eviction.md)). The URL is the cache key.

The generator makes one type per block, for example `MomentVideo`, `Avatar` and `VoiceNote`, each with the fields you named.

## Using the generated types

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

A field you marked optional in the file (`cover`, `blurhash`) is optional in the type.

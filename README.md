# MediaLayer

Images, video and audio for Swift and Kotlin. You describe your media sources once, MediaLayer fetches, streams, caches and prefetches them, and hands your app ready URLs and requests. Native players and image decoders stay yours.

One Rust core serves iOS and Android. It contains a local caching HTTP proxy, a single disk cache with a configurable eviction strategy, prefetch and the policy for a pool of players. Swift and Kotlin only get thin bindings and a typed API generated from your media files.

## Highlights

- **One fetch path.** Images, video (HLS and files) and audio go through the same core. You never see the proxy, its port, cache keys, deduplication or prefetch.
- **Native players.** AVPlayer and Media3 play from a local address. MediaLayer does not render or decode video.
- **Declare, don't command.** You say where video, an image or audio goes; the library picks a free invisible player, prepares it and swaps it in.
- **Player pool.** A pool of native players that the library juggles: it assigns sources, prerolls neighbors, hands playback over and recycles slots.
- **Image loaders stay.** Nuke on iOS and Coil on Android keep decoding, resizing and memory caching. Their network access and disk cache use the MediaLayer core.
- **Everything is configurable.** Budget, minimum size, prefetch windows, depth, timeouts and concurrency are settings with defaults, not constants.
- **Eviction strategy.** Choose `lru`, `fifo`, `lfu`, `largest_first` or `expire_after`. When the budget is reached, the cache shrinks to a configured share of it. Entries in use are never evicted.
- **Never blocks.** Public calls are cheap or asynchronous, network and disk run in parallel on library threads, and optional work waits for idle time.
- **Memory under control.** Player slots are recycled and freed completely, memory pressure shrinks everything, and a low-end profile keeps weak Android devices alive.
- **One core, two platforms.** All logic is Rust. Swift and Kotlin contain no business logic.

## Documentation

- [Architecture](docs/architecture.md)
- [Limits and eviction](docs/limits-and-eviction.md)
- [Threading and performance](docs/threading.md)
- [Declaring media](docs/declaring-media.md)
- [Player pool](docs/player-pool.md)
- [Memory control](docs/memory.md)

## Build and check

```
bash scripts/check_medialayer.sh
```

Runs Rust tests, builds the xcframework and runs Swift tests, builds libraries for 4 Android ABIs and runs Kotlin tests.

# Cache strategies

A strategy says how the library caches and preloads one kind of source. You declare it once in the `.mdl` file. The app writes no download, cancel or cleanup code: the library owns the whole life of the load.

| Strategy | Meaning |
| -------- | ------- |
| `window` (default) | Loads a few leading chunks of the sources around the visible page (`prefetch from:, to:`). Everything else loads when it is shown |
| `whole` | Downloads every part after declaration, independently of visibility. Playback uses the proxy and its cached parts |
| `on_demand` | Never part of the prefetch window. Loads only when a place shows or plays the source. The player pool still prepares the neighbours inside its `pool reach:` |

The names are the same in the `.mdl` file, in Swift (`CacheStrategy.window`, `.whole`, `.onDemand`) and in Kotlin (`CacheStrategy.WINDOW`, `.WHOLE`, `.ON_DEMAND`). Both platforms behave the same and are covered by the same cases.

## Declaring

`strategy` is an option of `video`, `image` and `audio` blocks. The generated source type carries it, so `MediaVideo`, `MediaImage` and `MediaAudio` need no extra argument.

```ruby
video :moment_circle do
  strategy :whole
end

audio :voice_note do
  strategy :whole
end

image :avatar do
  strategy :on_demand
end
```

Swift and Kotlin use the generated types like any other source. The strategy travels with the source, so there is nothing else to call:

### Swift

```swift
MediaVideo(MomentCircle(url: moment.circleUrl), playing: isActive)
```

### Kotlin

```kotlin
MediaVideo(source = MomentCircle(url = moment.circleUrl), playing = isActive)
```

A source is declared when a place shows it (`visible`) or when it is in a list given to `replaceWindow` or `replaceImageWindow`. The library looks at every item of the list, not only at the items around the active page, and starts a `whole` load for each `whole` item once. A `whole` or `on_demand` item is never part of the prefetch window, so it takes no window slot and no chunk budget.

## How `whole` loads

| Source | Load |
| ------ | ---- |
| Video and audio file | Every 1 MiB block in order. The first block gives the total length |
| Image | One request for the whole file |
| HLS | The playlist of the first variant (the library keeps one variant), then every segment and init segment of it |
| `file://` | Nothing. A local file is never fetched |

- **Dedup.** One load per source. A second declaration while it runs, or after it finished, does nothing. A place that starts playing the same source shares the block downloads that are already running, and the window never repeats a block that is stored.
- **Queues.** Blocks use the prefetch lane `slots` (video, audio, HLS) or `image_slots` (images), so the reserved playback permit and the playing source never wait for them. Whole videos and audios load one at a time in the order they were declared, whole images two at a time. A foreground playback does not cancel a `whole` load: the player reads the blocks that are stored and shares the ones that are running.
- **Cancel.** Moving the window, scrolling and hiding the place do not cancel it. `purge(url)` and `clear` make the library forget what it loaded, so the next declaration loads the source again. The library stops all loads when the app goes to the background and starts the unfinished ones again in the foreground. Closing the library cancels all loads.
- **Restart and resume.** Every finished block is on disk. A block cut by a restart is dropped. The next declaration skips the stored blocks and loads only the missing ones, so a half-loaded file resumes where it stopped.
- **Errors.** A failed block is tried 3 times, with waits of 0.5 and 1 second between the tries. Then the load stops and keeps what it has. It starts again at the next declaration or when the app returns to the foreground. There is no timer.
- **Disk.** Eviction removes ordinary entries before stored `whole` parts. Each group uses the configured [eviction strategy](limits-and-eviction.md). Completion checks every part. Missing parts fail the load. Protection belongs to stored entries and ends when those entries are removed.
- **Admission.** Pending whole declarations are limited by `dedupeMaxEntries`. Excess declarations receive a failed completion and can be declared again later.
- **Promotion.** Declaring a window source as whole downloads its remaining parts, including when its window load is still running.
- **Reset.** Clear and purge cancel pending work and discard the remembered plan, byte accounting and completion state.
- **Memory.** A `whole` load writes to disk in chunks and holds the same bounded proxy buffers as any download. The memory ceiling and the pool are unchanged.

## Platforms

Swift and Kotlin differ in no case: the names, the behaviour and the list of tested cases are the same.

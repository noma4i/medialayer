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

A source is declared when a place shows it (`visible`) or when it is in a list a window object received through `replace` or `replace_pages`. The library looks at every item of the list, not only at the items around the active page, and starts a `whole` load for each `whole` item once. A `whole` or `on_demand` item is never part of the prefetch window, so it takes no window slot and no chunk budget.

## Declare whole sources without a window

An app that wants sources stored in full, with no place and no list on screen, opens one declaration object and gives it the sources. The object belongs to one app screen or one feed. The library does the rest:

- It stores every source it is given in full, whatever strategy the source itself carries: the declaration sets `whole`. It takes only `https` and `http` sources. A `file://` source, an empty URL and another scheme are ignored.
- It loads a source once, however many declarations and windows hold it. The load starts when the library is ready, even when the declaration was made before. It does not depend on what is on screen.
- It uses the same loader as every `whole` load, so blocks are retried and resumed the same way and the same queue limits apply.
- It cancels a load when the last declaration or window that holds the source lets go, when the object is released or dropped, and when the library closes.

The app keeps its own choice of data. A chat declares only the messages stored after it started to watch the conversation, so the history that was already there is never downloaded. The app opens the declaration after the first snapshot of the rows and gives it the sources of the rows that arrive later.

### Swift

```swift
let hold = MediaWholeHold()
hold.add(newItems, in: media)
hold.release()
```

### Kotlin

```kotlin
val hold = MediaWholeHold()
hold.add(media, newItems)
hold.release()
```

`MediaWholeHold.add` gives the sources that arrived. `declare` gives the full list: the hold compares it with the previous one, adds the sources that are new and removes the ones that left. In a view tree, `mediaWholeDeclared` (Swift) and `DeclareMediaWhole` (Kotlin) open the declaration when the view appears and release it when the view goes away.

## How `whole` loads

| Source | Load |
| ------ | ---- |
| Video and audio file | Every 1 MiB block in order. The first block gives the total length |
| Image | One request for the whole file |
| HLS | The playlist of the first variant (the library keeps one variant), then every segment and init segment of it |
| `file://` | Nothing. A local file is never fetched |

- **Dedup.** Loads use the disk key: URL plus the configured header namespace. Different namespaces load independently in whole and window requests. A repeated declaration of the same key shares pending work or skips completed work. Playback shares stored and pending blocks with the same key.
- **Queues.** Blocks use the prefetch lane `slots` (video, audio, HLS) or `image_slots` (images), so the reserved playback permit and the playing source never wait for them. Whole videos and audios load one at a time in the order they were declared, whole images two at a time. A foreground playback does not cancel a `whole` load: the player reads the blocks that are stored and shares the ones that are running.
- **Cancel.** Moving the window, scrolling and hiding the place do not cancel it. `purge(url)` and `clear` make the library forget what it loaded, so the next declaration loads the source again. The library stops all loads when the app goes to the background and starts the unfinished ones again in the foreground. Releasing a declaration cancels the loads that no other declaration or window holds. Closing the library cancels all loads.
- **Restart and resume.** Every finished block is on disk. A block cut by a restart is dropped. The next declaration skips the stored blocks and loads only the missing ones, so a half-loaded file resumes where it stopped.
- **Errors.** A failed block is tried 3 times, with waits of 0.5 and 1 second between the tries. Then the load stops and keeps what it has. It starts again at the next declaration or when the app returns to the foreground. There are no background retries.
- **Timeout.** Writing bytes or completing a block resets the wait limit. A stalled load reports `TimedOut` after its retries and retains completed blocks. Total download duration is not limited.
- **Disk.** Eviction removes ordinary entries before stored `whole` parts. Each group uses the configured [eviction strategy](limits-and-eviction.md). Completion checks every part. Missing parts fail the load. Protection belongs to stored entries and ends when those entries are removed.
- **Admission.** Pending whole declarations are limited by `dedupeMaxEntries`. Excess declarations receive a failed completion and can be declared again later.
- **Promotion.** Declaring a window source as whole downloads its remaining parts, including when its window load is still running.
- **Reset.** Clear and purge cancel pending work and discard the remembered plan, byte accounting and completion state.
- **Memory.** A `whole` load writes to disk in chunks and holds the same bounded proxy buffers as any download. The memory ceiling and the pool are unchanged.

## Platforms

Swift and Kotlin differ in no case: the names, the behaviour and the list of tested cases are the same.

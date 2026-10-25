# Threading and performance

Сеть и сборка ядра работают в потоках библиотеки. Загрузка нативной библиотеки, `NativeCallbacks.install()`, чтение defaults и вызов ядра идут вне потока вызова: Kotlin на `Dispatchers.Default`, Swift в отдельной задаче.
Android создаёт платформенные объекты и каталог кэша на потоке вызова, подписку на Lifecycle ставит на главный поток. `open` возвращает `Deferred<MediaLayer>` (Swift: `async`), приложение ждёт его через `MediaLayerProvider` или `await`. Порядок объявлений не меняется: они идут после готовности слоя. Возврат не ждёт завершения `assemble` ядра.

Android Trace отмечает `MediaLayer.open.platform` на потоке вызова, `MediaLayer.open.jna`, `MediaLayer.open.defaults` и вложенный `MediaLayer.open.ffi` на потоке библиотеки.
Секции закрываются при исключении. Системная трассировка управляет записью, библиотека не включает её самостоятельно.

| Guarantee | Meaning |
| --------- | ------- |
| Calls are cheap | Calls like `prefetch` and `cancel` only queue work |
| Many requests at once | The proxy serves many players in parallel; one slow response never delays another |
| Disk never blocks | Cache reads and writes run off the threads that serve requests |
| Active playback first | Loading what is playing always goes before prefetch |
| Deferred work | Eviction, checks and far prefetch wait for idle time |
| Instant open | Opening does not scan the disk |
| Listeners | Called from library threads with the latest state only; hop to the main thread yourself |
| Cancel | Releases the caller's work and subscriptions. A shared download continues while another caller needs it |

UNBUILT: deferred work. Eviction starts right after a write that reaches the budget, the library has no idle signal yet, and the far prefetch window is not delayed.

Throwing Swift calls and Kotlin suspend calls forward cancellation to Rust. Non-throwing Swift calls finish normally when their task is cancelled.

On iOS native player creation and access run on the main thread. On Android every slot runs its ExoPlayer on its own library thread `medialayer-player-<slot>`: slot commands, listeners and video output changes run there, and the main thread never waits for a video output change. `ViewHandle.player` and `SlotPlayers.player` give a `Player` that is safe to call from the main thread: video output calls are posted to the player thread and the state that `ContentFrame` reads is a snapshot taken on that thread. ExoPlayer never sees a view or a holder. It gets only the frame target that MediaLayer hands it on the player thread, and the holder and texture callbacks run in MediaLayer on the main thread. Leaving the video with a live view only posts the clear. A destroyed `TextureView` returns `false` from `onSurfaceTextureDestroyed`, and MediaLayer releases the texture after the player confirms the clear, so the main thread does not wait. A destroyed `SurfaceView` is the one place where the main thread waits: the platform requires `surfaceDestroyed` to return only when nothing uses the frame target, so it returns after the player confirms the clear. Slot samples use the latest player events and elapsed time without a periodic observer. Sampling never waits for player creation.

### Swift

```swift
for await stats in media.statsStream() {
  await MainActor.run { self.cacheBytes = stats.totalBytes }
}
```

### Kotlin

```kotlin
media.statsFlow().collect { stats ->
  withContext(Dispatchers.Main) { cacheBytes = stats.totalBytes }
}
```

### Kotlin: raw bridge and inline dispatchers

`MediaLayer.open(context, ...)` from the `ui` module calls `NativeCallbacks.install()` for you. Call it yourself once, before the first `MediaLayer.open(OpenConfig)`, only when you use the generated bridge without `ui`.

It keeps library threads attached to the JVM after a native callback. Without it, a suspend call such as `stats()` or `playUrl()` resumed inline on a library thread (`Dispatchers.Unconfined`, a test dispatcher that does not dispatch) detaches that thread under its own frames, and Android aborts with "attempting to detach while still running code".

`MediaImage` makes no `imageStats` call on the main thread: the counters are taken once per environment (Kotlin `MediaEnvironment`, Swift `MediaContext`). The cache key stays a core rule. Kotlin passes it lazily and Coil asks for it once per request on its main-immediate interceptor, Swift asks once per view in `PreparedImage.init`; both keep the synchronous memory hit on the first frame, which needs the key at once.

The cache key is the only core call `MediaImage` makes per image on the main thread, and Kotlin memoizes it on the request data so the interceptor and the keyer share one call. Everything else leaves the main thread through bounded objects: a mounted place sends its latest `PlaceInput` through its own latest-wins drain (Swift `PlaceMount`, Kotlin `PlaceMount`) with at most one running block and one pending value on the shared concurrent executor (Swift global queue, Kotlin `Dispatchers.Default`), so a held mapping of one window never delays the release of another place. A close is terminal: it runs before any queued update, even when the update was refused, a window keeps one running preparation and one latest snapshot, and image counters merge into totals that one drain applies. No task or closure is queued per record. `imagePixelSize` stays a core rule: each environment keeps the last 64 results by their exact arguments (Kotlin `PixelSizeMemo` on `MediaEnvironment`, Swift `PixelSizeMemo` on the places object), so tiles of one size cost one call and the first value always comes from the core.

## Typed input and ownership

Every declaration enters the core through one typed, bounded input and one executor, before and after the library is assembled. The executor never runs on the calling thread.

- **Owners.** A `MediaPlace` (one mounted item, never reused), a `MediaWindow`, a subscription and a suspension each have a unique id. Closing an owner (`release` or drop) is idempotent and needs no free place: it marks the owner closed, hands its queued entries to the executor for freeing, and wakes it.
- **Aggregation.** Places of one key combine in the core: visible, playing and peeking are OR, groups are the set of playing groups, muted is AND over the places that gave an opinion, pin lives while any place pins. The source of a key is the last accepted declaration of a live visible place; its leaving restores the previous one. `rate` and `view_token` come from the last explicit intent of the best live place (playing, then visible, then newest). The app and the platform wrappers never repeat this policy.
- **Admission.** Entries, owners and retained bytes are limited (`OpenConfig.ingress`, defaults 256, 256 and the response limit). Bytes are weighed by backing capacity: slots, `None` entries, empty pages, strings, headers and failure details. A refused call returns `MediaError.Overloaded`, a call after closing returns `Closed`. Nothing is dropped silently. The reservation moves with the data to the executor-owned record and lives until the record is replaced or released.
- **Folding.** An update may replace the earlier snapshot of the same owner only until the next barrier. An event (seek of a place, player event) or a query is a barrier. Seeks by key of one key fold into the last fraction until the barrier.
- **Queries.** A request is ordered behind every accepted declaration. The executor fixes its logical result at that point (the route, the pool snapshot) and starts any IO as work owned by the request. The external network and disk are not part of an atomic snapshot with the pool. Cancelling a request or closing the library cancels its IO.
- **Controls.** `set_limits`, `reset_limits`, `clear`, `purge`, `evict_now`, `limits` and `stats` use their own bounded order and never wait for media events. `stats` takes its pool snapshot at a media barrier.
- **Subscriptions.** `subscribe_state`, `subscribe_stats` and `subscribe_tone` return a result and reserve their place before returning. A listener receives at most one terminal `failed(error)`: when the assembly fails, when the input overflows, or when the library closes. A cancelled subscription gets none.
- **Native events.** A flood of player events that exceeds admission closes the library with `Overloaded` and tells the observers; it never loses an event silently.
- **Closing.** Closing cancels and awaits the executor, signal work, tone preparation, the stats feed and the platform slots. A tone preparation that is still running is cancelled and its result is never delivered after `Closed`.

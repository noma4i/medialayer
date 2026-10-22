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

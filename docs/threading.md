# Threading and performance

MediaLayer never blocks your threads. Every public call either returns at once or is `async` (Swift) / `suspend` (Kotlin). Network, disk, parsing and eviction run on library threads, in parallel, and low-priority work is deferred until the app is idle.

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

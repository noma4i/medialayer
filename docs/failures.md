# Failures

The library reports every media load failure and every cache store failure on one stream. The app forwards it to its error tracker.

## What is reported

| Field | Meaning |
| ----- | ------- |
| `kind` | `Image`, `Video` or `Audio` |
| `url` | The source URL |
| `stage` | `Load`: the origin failed (timeout, network error, status). `Store`: the cache could not keep the response (budget reached with nothing to evict, disk full, write error) |
| `reason` | Short text of the cause |

A store failure never leaves a view blank. The loaded bytes are still delivered: an image is shown from memory, a video plays from the network. The event only tells you the cache did not keep them.

Each failed load is reported once, however many views wait for it.

## Swift

```swift
Task {
  for await failure in media.failureStream() {
    Sentry.capture(message: "\(failure.stage) \(failure.kind) \(failure.url): \(failure.reason)")
  }
}
```

## Kotlin

```kotlin
scope.launch {
  media.failureFlow().collect { failure ->
    Sentry.captureMessage("${failure.stage} ${failure.kind} ${failure.url}: ${failure.reason}")
  }
}
```

Ending the iteration or cancelling the collection cancels the watch. Events that happen before the watch starts are not replayed.

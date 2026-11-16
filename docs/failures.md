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

## Device and file failures

A second stream, `Media.failures`, reports every unexpected error of the platform: the audio session, the audio focus, the call speaker, a device step, the voice recorder, the camera and the removal of a file. Each event is a `NativeFailure`:

| Field | Meaning |
| ----- | ------- |
| `source` | `FailureSource`: `session`, `focus`, `speaker`, `device`, `recorder`, `camera` or `file` |
| `domain` | The system error domain (Swift `NSError.domain`) or the exception class (Kotlin) |
| `code` | The system error code, 0 or -1 when the platform gives none |
| `message` | The system text of the error |

One error gives one event. A refusal that the library turns into a state is not a failure: iOS `cannotInterruptOthers`, `insufficientPriority` and `cannotStartPlaying`, Android focus `FAILED` and `DELAYED`, and closing MediaLayer. A file that MediaLayer could not delete (a refused recording, a capture nobody took) is a `file` event. A flash or zoom command that the camera refused is a `camera` event, and `CameraState` keeps the old flash and zoom. `Media.failure` keeps the last error for a view that shows it.

### Swift

```swift
Task {
  for await failure in Media.failures {
    Sentry.capture(message: "\(failure.source) \(failure.domain) \(failure.code): \(failure.message)")
  }
}
```

### Kotlin

```kotlin
scope.launch {
  Media.failures.collect { failure ->
    Sentry.captureMessage("${failure.source} ${failure.domain} ${failure.code}: ${failure.message}")
  }
}
```

Events that happen while nobody listens are not replayed.

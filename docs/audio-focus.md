# Audio focus

MediaLayer can own the sound policy of your players: whether another app's music keeps playing, gets quieter, or stops while a place plays. You choose the policy once, when you call `open`. The app writes no audio session code for playback.

| Question | Answer |
| -------- | ------ |
| Who chooses the policy | The app, once, in `open` |
| Who talks to the platform | MediaLayer. On iOS one private actor owns every `AVAudioSession` call, none runs on the main thread |
| Default | `mix`: your sound plays beside other apps and never stops or lowers their music |
| Does `muted` change the policy | No. A muted place never takes the focus. Mute and focus are separate |

## Policies

| Policy | Other app's music | iOS session | Android focus |
| ------ | ----------------- | ----------- | ------------- |
| `exclusive` | Stops while a place plays | `playback`, mode `moviePlayback`, no options | Media3 requests `AUDIOFOCUS_GAIN` |
| `mix` | Keeps playing at full volume | `playback`, mode `moviePlayback`, `mixWithOthers` | No request. The player does not hold the focus |
| `duck` | Gets quieter while a place plays, then returns | as `mix` plus `duckOthers` | MediaLayer requests `AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK` for the first heard place and abandons it 300 ms after the last one. The player does not hold the focus |

Use `mix` when your app must never interrupt the user. MediaLayer writes nothing to the audio session while no place is heard, so a category your app set stays as it is. Use `duck` only when the user should hear your sound over the music.

## When a place takes the focus

A place takes the focus only while its player is heard. A player is heard when it is not muted, it is playing or waiting to play, and it holds an item.

| Place | Takes the focus |
| ----- | --------------- |
| Video or audio that plays with sound | Yes |
| Video that plays muted, or the neighbour page of a pager that peeks | No |
| Paused, ended or unloaded player | No |
| Image or GIF | No |

When the last heard player stops, MediaLayer ends the session and tells the other apps. That is what makes ducked music come back. MediaLayer waits 300 ms before it releases, so a pager that pauses one page and starts the next never lets the music back up in between.

## Choose the policy

### Swift

```swift
let media = try await MediaLayer.open(
    fetcher: fetcher,
    settings: settings,
    audioFocus: .mix
)
```

### Kotlin

```kotlin
val media: Deferred<MediaLayer> = MediaLayer.open(
    context, fetcher, settings,
    audioFocus = MediaAudioFocus.Mix
)
```

## Record

Recording has its own call and its own plan. It does not follow the playback policy, and it never mixes or ducks.

| Question | Answer |
| -------- | ------ |
| What does recording set | iOS: category `playAndRecord`, mode `default`, option `defaultToSpeaker`. Android: `AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE` |
| When | While a `holdRecording` token is live |
| What does releasing the token do | Ends the recording session. The next heard place sets the playback category again |
| Does the playback policy change recording | No. Recording works with every policy |
| Returns | A token, or `nil` on iOS and `null` on Android when the system refuses recording focus |

### Swift

```swift
guard let hold = await Media.audio.holdRecording() else { return }
// record
hold.release()
```

### Kotlin

```kotlin
// Request RECORD_AUDIO and record with your own recorder as before. The library only holds the focus.
val hold = Media.audio!!.holdRecording() ?: return
// record
hold.close()
```

On Android recording asks for `AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE`, so other apps pause while you record. Closing the token abandons it after 300 ms and the playback focus of the policy returns when a place is heard.

## Call

A call takes priority over recording, audible places and volume readers. On iOS its plan uses `playAndRecord`, `voiceChat` and `allowBluetoothHFP`. Speaker routing adds `defaultToSpeaker`. CallKit-managed calls let the system activate the session. On Android a call requests `AUDIOFOCUS_GAIN` with voice communication attributes and sets communication mode and routing. Ending the call restores the current demand. See [Calls](declaring-media.md#calls) for the API and WebRTC order.

## Read the volume

`readVolume` gives the current system volume and a stream of the changes, for example to unmute when the user presses volume up. While the stream lives, MediaLayer keeps a session that mixes and never ducks, even under `duck`. End the stream and the session goes back to idle.

Kotlin gives the volume of `STREAM_MUSIC` as a value from 0 to 1, as Swift does. `readVolume` returns null when the system reports no volume range. The system observer is registered when the stream is collected and removed when the collector is cancelled. No polling runs and no audio focus is requested.

### Swift

```swift
guard let reading = await Media.audio?.readVolume() else { return }
for await volume in reading.changes { /* react */ }
```

### Kotlin

```kotlin
val reading = Media.audio!!.readVolume() ?: return
val initial = reading.initial
reading.changes.collect { volume -> /* react */ }
```

## React to volume up

`volumeRaises` gives one event for each rise of the system volume, for example to unmute when the user presses volume up. A fall, or a rise of 0.001 or less, gives no event. The core holds this one rule, so both platforms agree. The system observer is on while the stream is read and off when it ends. No polling runs.

### Swift

```swift
guard let raises = await Media.audio?.volumeRaises() else { return }
for await _ in raises { Media.muted = false }
```

### Kotlin

```kotlin
Media.audio!!.volumeRaises().collect { Media.muted = false }
```

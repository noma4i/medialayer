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
let media = try MediaLayer.open(
    fetcher: fetcher,
    settings: settings,
    audioFocus: .mix
)
```

### Kotlin

```kotlin
val media = MediaLayer.open(
    context, fetcher, settings,
    audioFocus = MediaAudioFocus.Mix
)
```

## Record

Recording has its own call and its own plan. It does not follow the playback policy, and it never mixes or ducks.

| Question | Answer |
| -------- | ------ |
| What does recording set | iOS: category `playAndRecord`, mode `default`, option `defaultToSpeaker`. Android: `AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE` |
| When | Only between `activateRecording` and `deactivateRecording` |
| What does `deactivateRecording` do | Ends the recording session. The next heard place sets the playback category again |
| Does the playback policy change recording | No. Recording works with every policy |
| Returns | `false` when the system refuses the recording focus. The call leaves no recording demand then |

### Swift

```swift
guard await Media.audio.activateRecording() else { return }
// record
await Media.audio.deactivateRecording()
```

### Kotlin

```kotlin
// Request RECORD_AUDIO and record with your own recorder as before. The library only holds the focus.
if (!Media.audio!!.activateRecording()) return
// record
Media.audio!!.deactivateRecording()
```

On Android recording asks for `AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE`, so other apps pause while you record. `deactivateRecording` abandons it after 300 ms and the playback focus of the policy returns when a place is heard.

## Read the volume

`readVolume` gives the current system volume and a stream of the changes, for example to unmute when the user presses volume up. While the stream lives, MediaLayer keeps a session that mixes and never ducks, even under `duck`. End the stream and the session goes back to idle.

### Swift

```swift
guard let reading = await Media.audio?.readVolume() else { return }
for await volume in reading.changes { /* react */ }
```

### Kotlin

```kotlin
// Android: observe the system volume with the platform API. No focus is needed.
```

# Audio focus

MediaLayer owns the sound of your app: the audio session on iOS and the audio focus on Android, for playback, voice and video recording, and calls. You choose the playback policy once, when you call `open`. The app writes no audio session or audio focus code.

| Question | Answer |
| -------- | ------ |
| Who decides | One rule in the core, the same on iOS and Android. The platforms only pass system events in and run what the rule answers |
| Who talks to the platform | MediaLayer. No `AVAudioSession` or `AudioManager` call runs on the main thread |
| Default | `mix`: your sound plays beside other apps and never stops or lowers their music |
| Does `muted` change the policy | No. A muted place never takes the focus. Mute and focus are separate |
| Does a lost headphone change `Media.muted` | No. `Media.muted` is only the user's flag |

## Policies

| Policy | Other app's music | iOS session | Android focus |
| ------ | ----------------- | ----------- | ------------- |
| `exclusive` | Stops while a place plays | `playback`, mode `moviePlayback`, no options | One `AUDIOFOCUS_GAIN` request for the app |
| `mix` | Keeps playing at full volume | `playback`, mode `moviePlayback`, `mixWithOthers` | No request |
| `duck` | Gets quieter while a place plays, then returns | as `mix` plus `duckOthers` | One `AUDIOFOCUS_GAIN` request for the app |

On Android MediaLayer holds one focus request for the whole app. The players never request focus themselves, so two players can never take the focus from each other.

## When a place takes the focus

A place takes the focus only while its player is heard. A player is heard when it really plays (iOS `timeControlStatus` is playing, Android `isPlaying`), it is not muted, and it holds an item.

| Place | Takes the focus |
| ----- | --------------- |
| Video or audio that plays with sound | Yes |
| Video that plays muted, or the neighbour page of a pager that peeks | No |
| Paused, ended or unloaded player | No |
| Image or GIF | No |

When the last heard player stops, MediaLayer ends the session and tells the other apps, so ducked music comes back. On iOS the session then rests in the playback category of your policy, so a player that starts later always plays through the right output.

## What the system does to playback

| Event | What MediaLayer does |
| ----- | -------------------- |
| An interruption (an alarm, Siri, a call screen) or a temporary loss of focus | Pauses every playing place |
| The interruption ends and the system allows a resume | Plays the same places again |
| The interruption ends without that permission, or the app returns to the foreground while it was interrupted | Plays nothing until a place declares play again |
| Another app takes the focus for good (Android) | Pauses, forgets the play wish, and asks for focus only on a new play |
| A request is ducked (Android) | Plays at 20 percent volume until the focus returns |
| The previous route had a private output and the new route has none | Pauses every playing place at any time. `Media.muted` does not change. A switch from A2DP to HFP is not a loss |
| The media services reset (iOS) | Drops every player and plays nothing until a place declares play again |

A refused or delayed request leaves that state on a system event or a new demand. A newly sounding place, a new recording, a new call, or a new volume reader is a new demand.

Unexpected session, focus and speaker errors reach `Media.failure` with their system domain and code. iOS refusals `cannotInterruptOthers`, `insufficientPriority` and `cannotStartPlaying` are states, not failures. Android focus `FAILED` and `DELAYED` are also states. Closing MediaLayer is not a failure.

## Record

A voice recording or a video recording has its own session. It does not follow the playback policy, and it never mixes or ducks.

| Question | Answer |
| -------- | ------ |
| What does recording set | iOS: `playAndRecord`, mode `default` for voice and `videoRecording` for video, option `allowBluetoothA2DP` (headphones stay the output). Android: `AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE` |
| Playback during a recording | Paused when the recording starts. The end of the recording plays nothing back |
| When is a recording admitted | Only after the session for it is in place. If the system refuses it, the start fails with `microphoneBusy` and no microphone is taken |
| How many recordings | One of each kind. A camera and a voice recorder can hold the microphone together |
| Camera | A camera with sound holds its recording session from the preview on |
| What stops a running recording | A call, an interruption, a lost focus, a denied permission, a silenced microphone. The recording ends with that refusal and its file is deleted |

The refusals are `VoiceRefusal` and `CameraRefusal` cases with their text from the core: `inCall`, `interrupted`, `microphoneSilent`, `microphoneBusy`, `permissionDenied`. A video take with sound starts only while the microphone sound flows, and a take whose sound stops is thrown away with `microphoneSilent`: a silent clip never reaches the app.

## Call

A call has the highest priority. While any call runs, every player pauses, a place that declares play gets no play, `playUrl` fails with `MediaError.InCall`, and recordings stop or are refused with `inCall`. The end of a call restores nothing: a place plays after its owner declares play again. See [Calls](declaring-media.md#calls) for the API.

| Call | iOS | Android |
| ---- | --- | ------- |
| Your CallKit call | MediaLayer releases its session and only sets `playAndRecord` with `voiceChat`. CallKit activates it | - |
| Your Core-Telecom call | - | Telecom owns focus, mode and route. MediaLayer only pauses and refuses recording |
| Your own call without the system | MediaLayer activates `playAndRecord` with `voiceChat` | MediaLayer requests `AUDIOFOCUS_GAIN_TRANSIENT` and sets the communication mode |
| A call of the phone or of another app | Seen through `CXCallObserver` | Seen through the phone call state and the communication mode of another app |

Ending your own call clears its communication mode. A later communication mode belongs to another app.

## Read the volume

`readVolume` gives the current system volume and a stream of the changes, for example to unmute when the user presses volume up. While the stream lives and nothing is heard, MediaLayer keeps a session that mixes and never ducks. End the stream and the session goes back to idle. No polling runs.

### Swift

```swift
guard let reading = await Media.audio?.readVolume() else { return }
for await volume in reading.changes { /* react */ }
```

### Kotlin

```kotlin
val reading = media.readVolume() ?: return
reading.changes.collect { volume -> /* react */ }
```

## React to volume up

`volumeRaises` gives one event for each rise of the system volume. A fall, or a rise of 0.001 or less, gives no event. The core holds this rule, so both platforms agree.

### Swift

```swift
for await _ in await Media.audio?.volumeRaises() ?? AsyncStream { $0.finish() } { Media.muted = false }
```

### Kotlin

```kotlin
media.volumeRaises().collect { Media.muted = false }
```

## Choose the policy

### Swift

```swift
let media = try await MediaLayer.open(fetcher: fetcher, settings: settings, audioFocus: .mix)
```

### Kotlin

```kotlin
val media: Deferred<MediaLayer> = MediaLayer.open(context, fetcher, settings, audioFocus = MediaAudioFocus.Mix)
```

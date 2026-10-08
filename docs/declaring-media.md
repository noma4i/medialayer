# Declaring media

You declare what to show and where. MediaLayer does the rest.

| Kind | Declaration | What the library does |
| ---- | ----------- | --------------------- |
| Video | `MediaVideo(source)` | Picks a free, invisible player from the pool, prepares the source in it and puts it in place when ready; hands playback over while scrolling and recycles the player when you leave |
| Image | `MediaImage(source, size)` | Loads through the shared cache, deduplicates requests and prefetches the nearby window |
| Audio | `MediaAudio(source)` | Caches, plays and exposes `play`, `pause`, `seek` and `progress` |

All three share one state type, `MediaState`: `idle`, `loading`, `ready`, `failed`. A source is a generated type from your media file, so there are no URL strings or cache keys in app code. You only report whether a place is visible or active; prefetch, preroll, handoff and release follow from that.

## Playing and sound

Every place takes `playing` (true or false) and `muted`. A player that is not playing stays prepared and paused. `Media.muted` mutes the whole app. Muting changes only the volume of the player; it never touches the audio session, so other apps' music is not interrupted by a mute toggle.

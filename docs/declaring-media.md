# Declaring media

You declare what to show and where. MediaLayer does the rest.

| Kind | Declaration | What the library does |
| ---- | ----------- | --------------------- |
| Video | `MediaVideo(source)` | Picks a free, invisible player from the pool, prepares the source in it and puts it in place when ready; hands playback over while scrolling and recycles the player when you leave |
| Image | `MediaImage(source, size)` | Loads through the shared cache, deduplicates requests and prefetches the nearby window |
| Audio | `MediaAudio(source)` | Caches, plays and exposes `play`, `pause`, `seek` and `progress` |

All three share one state type, `MediaState`: `idle`, `loading`, `ready`, `failed`. A source is a generated type from your media file, so there are no URL strings or cache keys in app code. You only report whether a place is visible or active; prefetch, preroll, handoff and release follow from that.

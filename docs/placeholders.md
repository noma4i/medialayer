# Placeholders

While media loads, a place shows placeholders chosen by MediaLayer. You only declare the source.

| Kind | Chain |
| ---- | ----- |
| Image | BlurHash decoded from a short string (one decoder in the core for both platforms), then the image |
| Video with a cover | The cover while the video is not ready, then the video |
| Video without a cover | The first frame, cut from the first chunk and shown without starting playback, then the video |
| Video already cached or ready | Playback starts at once, no placeholder |

Videos never use BlurHash. For a stream, the first frame comes from the first chunk through the local proxy, and playback starts as soon as the player has a buffer. Each step replaces the previous one only when it is ready, so nothing flickers.

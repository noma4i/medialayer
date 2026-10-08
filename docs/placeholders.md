# Placeholders

While media loads, a place shows a chain of placeholders chosen by MediaLayer: BlurHash, then the cover image, then the first video frame, then playback.

| Step | Shown | Source |
| ---- | ----- | ------ |
| BlurHash | A blurred preview decoded from a short string | The hash from your server; one decoder in the core for both platforms |
| Cover | The poster image | The shared image path and cache |
| First frame | A still frame without starting playback | The first chunk of the stream, loaded ahead through the proxy |
| Video | Playback | The player from the pool |

Each step replaces the previous one only when it is ready, so nothing flickers. The depth of the first chunk and the BlurHash size are settings.

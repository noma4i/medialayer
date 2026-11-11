# Shape and grayscale

`MediaImage` and `MediaVideo` take their outline and `MediaImage` takes its colour mode from the SwiftUI environment. The library applies both on the `CALayer` that owns the contents. It adds no SwiftUI `clipShape` mask over sublayers and no `.grayscale` filter over a live `CALayer`.

| Modifier | Effect |
| --- | --- |
| `.mediaShape(.circle)` | `cornerRadius` is half of the shorter side, `masksToBounds` is on, on the `CALayer` that holds the image contents or on the `AVPlayerLayer` of the video view |
| `.mediaShape(.rounded(radius))` | The same with the given radius |
| `.mediaShape(.rectangle)` (default) | No rounding |
| `.mediaGrayscale()` | Each decoded frame or still image is converted to gray once, when it is decoded or first shown. A GIF or WebP converts its frames off the main thread, so the display link only sets `CALayer.contents` |

Swift: `MediaImage(source).mediaGrayscale().mediaShape(.circle)` and `MediaVideo(source).mediaShape(.circle)`.

`mediaGrayscale` has no effect on `MediaVideo`. A still image with the default shape and colour mode stays a SwiftUI `Image`. A still image with a shape or grayscale is drawn as the `CALayer.contents` of one view.

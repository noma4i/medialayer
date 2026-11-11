# Animated images

`MediaImage` plays an animated GIF or WebP on its own. The app passes one value, `ImageAnimation`, to say whether it plays, how often and what it shows when it rests.

| Field | Meaning |
| ----- | ------- |
| `playing` (default `true`) | `false` rests the image on its rest frame |
| `repeats` (default forever) | `.forever`, or `.times(n)` loops and then stops on the rest frame |
| `rest` (default first) | The frame shown when the image rests: `.first` or `.last` |
| `ignoresReduceMotion` (default `false`) | `true` keeps the image playing when the system asks to remove animations |

Swift: `MediaImage(source, animation: ImageAnimation(repeats: .times(1), rest: .last))`.
Compose: `MediaImage(source, animation = ImageAnimation(repeats = Repeats.Times(1), rest = RestFrame.Last))`.

## When the image rests

The image rests when `playing` is `false`, or when the system setting "remove animations" is on and `ignoresReduceMotion` is `false`. The library reads the setting and follows its changes: iOS `accessibilityReduceMotion`, Android `ANIMATOR_DURATION_SCALE` equal to zero. The app writes no code for it.

## Off screen

An image that is off screen, in a hidden window or in the background draws no frames, decodes nothing and wakes nothing. It continues when it is visible again.

## Decoding

- iOS decodes frames from the bytes the core delivered (`FrameStream`). A GIF or WebP over the byte budget keeps a window of three frames. Decoding runs off the main thread and ends when the view leaves.
- iOS presents a frame by setting `CALayer.contents` of the animation view on a display-link tick, one write per frame change on the main thread. A frame that is not decoded yet leaves the previous frame on screen. The display link runs only while the image plays, so an image that is off screen, rested or in the background has no link. There is no `UIImageView.image` write per frame.
- Android decodes through the Coil animated decoder and plays it with `AnimatedImageDrawable`. At a playback rate other than 1 it paints the frames off the main thread (a GIF with `Movie`, a WebP frame by frame) and shows each frame at the scaled time, because the platform drawable has no speed control.

## Playback rate

`MediaImage` plays an animated GIF or WebP at a given speed. The app passes `playbackRate`, a number above 0: 2 plays twice as fast and 0.5 half as fast. A value that is not above 0 plays at 1. A still image and a resting image ignore it.

Swift: `MediaImage(source, playbackRate: 0.5)`.
Compose: `MediaImage(source, playbackRate = 0.5)`.

The rate scales the clock only. Frame order, frame delays, `repeats`, the rest frame, the reduce-motion rest and the pause off screen stay as at rate 1. A new rate changes the speed of the playing image from the frame it shows. On Android a change between two rates other than 1 loads nothing again. A change between 1 and another rate decodes the image again from the library cache, because rate 1 plays the platform drawable and any other rate plays the library clock.

## Rest frame on Android

A resting image never shows the running animated drawable. `RestFrame.First` asks Coil for a still decode of the source. `RestFrame.Last` decodes the last frame off the main thread: an animated WebP is parsed by its RIFF chunks and composed frame by frame, a GIF is read with `Movie` at its end. A source that is not animated falls through to the next decoder. The image that finished its `repeats` rests on its rest frame the same way, so Swift and Compose show the same frame.

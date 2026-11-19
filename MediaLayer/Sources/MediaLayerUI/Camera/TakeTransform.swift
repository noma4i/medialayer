#if os(iOS)
import CoreGraphics
import ImageIO
import MediaLayer

/// The angle of the preview of the interface, the portrait (INV-CAPTURE-80). The preview connection gets it explicitly and a take uses the same constant,
/// so the recorded pixels match the preview by construction, in any pose of the device, whatever the defaults of AVFoundation are.
let previewAngle: CGFloat = 90

func takeTransform(mirror: Bool) -> CGAffineTransform {
  let turned = CGAffineTransform(rotationAngle: previewAngle * .pi / 180)
  return mirror ? turned.concatenating(CGAffineTransform(scaleX: -1, y: 1)) : turned
}

/// How the frames of the video output come out of the connection: turned upright (and mirrored for a front lens that mirrors its video) by the capture itself, or raw as the sensor gives them.
struct FrameTurn: Equatable, Sendable {
  var upright = false
  var mirrored = false
}

/// Upright frames carry no turn in the file; frames the connection could not turn keep the transform of the take (INV-CAPTURE-29).
func takeWriterTransform(turn: FrameTurn, mirror: Bool) -> CGAffineTransform {
  turn.upright ? .identity : takeTransform(mirror: mirror)
}

/// How a frame of the tap is turned against an upright picture. Upright frames are `.up`, and `.upMirrored` for a front lens whose video is not mirrored (the selfie view of the raw frame).
func tapOrientation(lens: CameraLens, turn: FrameTurn) -> CGImagePropertyOrientation {
  guard turn.upright else { return lens == .front ? .leftMirrored : .right }
  return lens == .front && !turn.mirrored ? .upMirrored : .up
}
#endif

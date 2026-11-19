#if os(iOS)
import CoreGraphics
import ImageIO
import MediaLayer

/// The angle of the preview of the interface, the portrait. Only the preview connection gets it: the library never turns or mirrors the recorded video (INV-CAPTURE-80).
let previewAngle: CGFloat = 90

/// How a frame of the tap is turned against an upright picture: the buffer is as the platform delivers it, the sensor orientation of the lens (a front lens is also a selfie view).
func tapOrientation(lens: CameraLens) -> CGImagePropertyOrientation {
  lens == .front ? .leftMirrored : .right
}
#endif

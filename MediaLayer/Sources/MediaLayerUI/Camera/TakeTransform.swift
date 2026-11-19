#if os(iOS)
import CoreGraphics

/// The angle of the preview of the interface, the portrait (INV-CAPTURE-80). The preview connection gets it explicitly and a take uses the same constant,
/// so the recorded pixels match the preview by construction, in any pose of the device, whatever the defaults of AVFoundation are.
let previewAngle: CGFloat = 90

func takeTransform(mirror: Bool) -> CGAffineTransform {
  let turned = CGAffineTransform(rotationAngle: previewAngle * .pi / 180)
  return mirror ? turned.concatenating(CGAffineTransform(scaleX: -1, y: 1)) : turned
}
#endif

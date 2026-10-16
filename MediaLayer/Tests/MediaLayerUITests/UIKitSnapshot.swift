#if canImport(UIKit)
import SwiftUI
import UIKit

@MainActor
func firstFrame(of view: some View, size: CGSize) -> CGImage? {
  let host = UIHostingController(rootView: view)
  host.view.backgroundColor = .clear
  host.safeAreaRegions = []
  let window = UIWindow(frame: CGRect(origin: .zero, size: size))
  window.rootViewController = host
  window.isHidden = false
  defer {
    window.isHidden = true
    window.rootViewController = nil
  }
  host.view.frame = window.bounds
  host.view.layoutIfNeeded()
  CATransaction.flush()
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  return UIGraphicsImageRenderer(size: size, format: format).image { context in
    host.view.layer.render(in: context.cgContext)
  }.cgImage
}
#endif

import QuartzCore
import SwiftUI

func layoutHosted(_ layer: CALayer?, bounds: CGRect) {
  CATransaction.begin()
  CATransaction.setDisableActions(true)
  layer?.sublayers?.forEach { $0.frame = bounds }
  CATransaction.commit()
}

final class SurfaceAttachment {
  let places: MediaPlaces
  let key: String
  let token: UInt64

  init(places: MediaPlaces, key: String, layer: CALayer) {
    self.places = places
    self.key = key
    token = HostLayers.shared.register(layer)
    places.attachView(key: key, token: token)
  }

  deinit {
    places.detachView(key: key)
    HostLayers.shared.unregister(token)
  }
}

#if canImport(UIKit)
import UIKit

final class SurfaceView: UIView {
  var attachment: SurfaceAttachment?

  override func layoutSubviews() {
    super.layoutSubviews()
    layoutHosted(layer, bounds: bounds)
  }
}

struct VideoSurface: UIViewRepresentable {
  let places: MediaPlaces
  let key: String

  func makeUIView(context: Context) -> SurfaceView {
    let view = SurfaceView()
    view.backgroundColor = .clear
    view.isUserInteractionEnabled = false
    return view
  }

  func updateUIView(_ view: SurfaceView, context: Context) {
    if view.attachment == nil {
      view.attachment = SurfaceAttachment(places: places, key: key, layer: view.layer)
    }
  }

  static func dismantleUIView(_ view: SurfaceView, coordinator: ()) {
    view.attachment = nil
  }
}
#else
import AppKit

final class SurfaceView: NSView {
  var attachment: SurfaceAttachment?

  override func layout() {
    super.layout()
    layoutHosted(layer, bounds: bounds)
  }
}

struct VideoSurface: NSViewRepresentable {
  let places: MediaPlaces
  let key: String

  func makeNSView(context: Context) -> SurfaceView {
    let view = SurfaceView()
    view.wantsLayer = true
    return view
  }

  func updateNSView(_ view: SurfaceView, context: Context) {
    if view.attachment == nil, let layer = view.layer {
      view.attachment = SurfaceAttachment(places: places, key: key, layer: layer)
    }
  }

  static func dismantleNSView(_ view: SurfaceView, coordinator: ()) {
    view.attachment = nil
  }
}
#endif

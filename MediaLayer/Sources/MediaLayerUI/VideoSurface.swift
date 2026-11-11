import AVFoundation
import QuartzCore
import SwiftUI

func layoutHosted(_ layer: CALayer?, bounds: CGRect, gravity: AVLayerVideoGravity = .resizeAspectFill, shape: MediaShape = .rectangle) {
  CATransaction.begin()
  CATransaction.setDisableActions(true)
  layer?.sublayers?.forEach {
    $0.frame = bounds
    $0.cornerRadius = shape.radius(in: bounds.size)
    $0.masksToBounds = $0.cornerRadius > 0
    ($0 as? AVPlayerLayer)?.videoGravity = gravity
  }
  CATransaction.commit()
}

final class SurfaceHost: @unchecked Sendable {
  let layer = CALayer()
  let token: UInt64
  private let hosts: HostLayers

  init(places: MediaPlaces) {
    hosts = HostLayers.of(places)
    token = hosts.register(layer)
  }

  deinit { hosts.unregister(token) }
}

#if canImport(UIKit)
import UIKit

final class SurfaceView: UIView {
  var host: SurfaceHost?
  var gravity = AVLayerVideoGravity.resizeAspectFill
  var shape = MediaShape.rectangle

  override func layoutSubviews() {
    super.layoutSubviews()
    host?.layer.frame = bounds
    layoutHosted(host?.layer, bounds: bounds, gravity: gravity, shape: shape)
  }
}

struct VideoSurface: UIViewRepresentable {
  let host: SurfaceHost
  var gravity = AVLayerVideoGravity.resizeAspectFill
  var shape = MediaShape.rectangle

  func makeUIView(context: Context) -> SurfaceView {
    let view = SurfaceView()
    view.backgroundColor = .clear
    view.isUserInteractionEnabled = false
    return view
  }

  func updateUIView(_ view: SurfaceView, context: Context) {
    if view.gravity != gravity || view.shape != shape {
      view.gravity = gravity
      view.shape = shape
      view.setNeedsLayout()
    }
    if view.host !== host {
      view.host?.layer.removeFromSuperlayer()
      view.host = host
      view.layer.addSublayer(host.layer)
      view.setNeedsLayout()
    }
  }

  static func dismantleUIView(_ view: SurfaceView, coordinator: ()) {
    view.host?.layer.removeFromSuperlayer()
    view.host = nil
  }
}
#else
import AppKit

final class SurfaceView: NSView {
  var host: SurfaceHost?
  var gravity = AVLayerVideoGravity.resizeAspectFill

  override func layout() {
    super.layout()
    host?.layer.frame = bounds
    layoutHosted(host?.layer, bounds: bounds, gravity: gravity)
  }
}

struct VideoSurface: NSViewRepresentable {
  let host: SurfaceHost
  var gravity = AVLayerVideoGravity.resizeAspectFill
  var shape = MediaShape.rectangle

  func makeNSView(context: Context) -> SurfaceView {
    let view = SurfaceView()
    view.wantsLayer = true
    return view
  }

  func updateNSView(_ view: SurfaceView, context: Context) {
    if view.gravity != gravity {
      view.gravity = gravity
      view.needsLayout = true
    }
    if view.host !== host {
      view.host?.layer.removeFromSuperlayer()
      view.host = host
      view.layer?.addSublayer(host.layer)
      view.needsLayout = true
    }
  }

  static func dismantleNSView(_ view: SurfaceView, coordinator: ()) {
    view.host?.layer.removeFromSuperlayer()
    view.host = nil
  }
}
#endif

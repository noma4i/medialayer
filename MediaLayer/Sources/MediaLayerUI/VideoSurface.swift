import AVFoundation
import QuartzCore
import SwiftUI

func layoutHosted(_ layer: CALayer?, bounds: CGRect, gravity: AVLayerVideoGravity = .resizeAspectFill) {
  CATransaction.begin()
  CATransaction.setDisableActions(true)
  layer?.sublayers?.forEach {
    $0.frame = bounds
    ($0 as? AVPlayerLayer)?.videoGravity = gravity
  }
  CATransaction.commit()
}

final class SurfaceAttachment {
  let places: MediaPlaces
  let key: String
  let token: UInt64
  private let hosts: HostLayers

  init(places: MediaPlaces, key: String, layer: CALayer, owner: UUID = UUID(), isPlaying: Bool = false, hosts: HostLayers? = nil) {
    let hosts = hosts ?? HostLayers.of(places)
    self.places = places
    self.key = key
    self.hosts = hosts
    token = hosts.register(layer)
    if hosts.claim(key: key, token: token, owner: owner, isPlaying: isPlaying) { places.attachView(key: key, token: token) }
  }

  deinit {
    hosts.unregister(token)
    switch hosts.release(key: key, token: token) {
    case .none: break
    case .detach: places.detachView(key: key)
    case .handover(let previous): places.attachView(key: key, token: previous)
    }
  }
}

#if canImport(UIKit)
import UIKit

final class SurfaceView: UIView {
  var attachment: SurfaceAttachment?
  var gravity = AVLayerVideoGravity.resizeAspectFill

  override func layoutSubviews() {
    super.layoutSubviews()
    layoutHosted(layer, bounds: bounds, gravity: gravity)
  }
}

struct VideoSurface: UIViewRepresentable {
  let places: MediaPlaces
  let key: String
  let owner: UUID
  let isPlaying: Bool
  var gravity = AVLayerVideoGravity.resizeAspectFill

  func makeUIView(context: Context) -> SurfaceView {
    let view = SurfaceView()
    view.backgroundColor = .clear
    view.isUserInteractionEnabled = false
    return view
  }

  func updateUIView(_ view: SurfaceView, context: Context) {
    if view.gravity != gravity {
      view.gravity = gravity
      view.setNeedsLayout()
    }
    if view.attachment == nil {
      view.attachment = SurfaceAttachment(places: places, key: key, layer: view.layer, owner: owner, isPlaying: isPlaying)
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
  var gravity = AVLayerVideoGravity.resizeAspectFill

  override func layout() {
    super.layout()
    layoutHosted(layer, bounds: bounds, gravity: gravity)
  }
}

struct VideoSurface: NSViewRepresentable {
  let places: MediaPlaces
  let key: String
  let owner: UUID
  let isPlaying: Bool
  var gravity = AVLayerVideoGravity.resizeAspectFill

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
    if view.attachment == nil, let layer = view.layer {
      view.attachment = SurfaceAttachment(places: places, key: key, layer: layer, owner: owner, isPlaying: isPlaying)
    }
  }

  static func dismantleNSView(_ view: SurfaceView, coordinator: ()) {
    view.attachment = nil
  }
}
#endif

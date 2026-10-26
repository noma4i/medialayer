#if os(iOS)
import AVFoundation
import MediaLayer
import SwiftUI
import UIKit

final class CameraPreviewLayerView: UIView {
  override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }

  var previewLayer: AVCaptureVideoPreviewLayer {
    guard let layer = layer as? AVCaptureVideoPreviewLayer else { preconditionFailure("the layer class is AVCaptureVideoPreviewLayer") }
    return layer
  }

  private let camera: MediaCamera
  private var attached = false
  private var lastScale: Double = 1
  private lazy var pinchRecognizer = UIPinchGestureRecognizer(target: self, action: #selector(handlePinch(_:)))

  var zoomsByPinch = true {
    didSet {
      guard zoomsByPinch != oldValue else { return }
      if zoomsByPinch { addGestureRecognizer(pinchRecognizer) } else { removeGestureRecognizer(pinchRecognizer) }
    }
  }

  init(camera: MediaCamera) {
    self.camera = camera
    super.init(frame: .zero)
    previewLayer.session = camera.previewSession
    previewLayer.videoGravity = .resizeAspectFill
    addGestureRecognizer(pinchRecognizer)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { nil }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    sync(onScreen: window != nil)
  }

  func sync(onScreen: Bool) {
    guard onScreen != attached else { return }
    attached = onScreen
    if onScreen { camera.attachPreview() } else { camera.detachPreview() }
  }

  @objc private func handlePinch(_ recognizer: UIPinchGestureRecognizer) {
    pinch(scale: Double(recognizer.scale), state: recognizer.state)
  }

  /// `zoom(by:)` takes a step against the current zoom, so a pinch sends the ratio of its scale to the scale of the last event.
  func pinch(scale: Double, state: UIGestureRecognizer.State) {
    switch state {
    case .began:
      lastScale = scale
    case .changed:
      camera.zoom(by: scale / lastScale)
      lastScale = scale
    default:
      break
    }
  }
}

/// The preview of a camera. The camera runs while this view is in a window and the app is in front. The picture fills the frame.
public struct MediaCameraPreview: UIViewRepresentable {
  private let camera: MediaCamera
  private let zoomsByPinch: Bool
  private let onLayer: ((AVCaptureVideoPreviewLayer) -> Void)?

  public init(camera: MediaCamera, zoomsByPinch: Bool = true, onLayer: ((AVCaptureVideoPreviewLayer) -> Void)? = nil) {
    self.camera = camera
    self.zoomsByPinch = zoomsByPinch
    self.onLayer = onLayer
  }

  public func makeUIView(context: Context) -> UIView {
    let view = CameraPreviewLayerView(camera: camera)
    view.zoomsByPinch = zoomsByPinch
    onLayer?(view.previewLayer)
    return view
  }

  public func updateUIView(_ view: UIView, context: Context) {
    (view as? CameraPreviewLayerView)?.zoomsByPinch = zoomsByPinch
  }

  public static func dismantleUIView(_ view: UIView, coordinator: ()) {
    (view as? CameraPreviewLayerView)?.sync(onScreen: false)
  }
}

/// Gives the state of a camera to `content` and follows it while the view is mounted, like `WithMediaTone`.
public struct WithMediaCameraState<Content: View>: View {
  private let camera: MediaCamera
  private let content: (CameraState) -> Content
  @State private var state: CameraState

  public init(camera: MediaCamera, @ViewBuilder content: @escaping (CameraState) -> Content) {
    self.camera = camera
    self.content = content
    _state = State(initialValue: camera.state)
  }

  public var body: some View {
    content(state).task {
      for await next in camera.states { state = next }
    }
  }
}
#endif

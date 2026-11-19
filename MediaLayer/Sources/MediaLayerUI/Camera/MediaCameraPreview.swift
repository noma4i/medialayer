#if os(iOS)
import AVFoundation
import MediaLayer
import SwiftUI
import UIKit

final class CameraPreviewLayer: AVCaptureVideoPreviewLayer {
  private(set) var sessionJoinsOnMain = 0

  override var session: AVCaptureSession? {
    get { super.session }
    set {
      if Thread.isMainThread, newValue != nil { sessionJoinsOnMain += 1 }
      super.session = newValue
    }
  }

  override func setSessionWithNoConnection(_ session: AVCaptureSession) {
    if Thread.isMainThread { sessionJoinsOnMain += 1 }
    super.setSessionWithNoConnection(session)
  }
}

final class CameraPreviewLayerView: UIView {
  /// The preview layer is a sublayer, not the backing layer: the backend joins it to the session on the session queue, and UIKit must not see that as a layout of the view off Main.
  let previewLayer = CameraPreviewLayer()
  private var previewingObservation: NSKeyValueObservation?

  private var camera: MediaCamera
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
    previewLayer.backgroundColor = UIColor.clear.cgColor
    previewLayer.videoGravity = .resizeAspectFill
    layer.addSublayer(previewLayer)
    previewingObservation = previewLayer.observe(\.isPreviewing, options: [.new]) { [weak self] _, change in
      let previewing = change.newValue ?? false
      DispatchQueue.main.async { self?.applyPreviewing(previewing) }
    }
    camera.registerPreview(previewLayer)
    addGestureRecognizer(pinchRecognizer)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { nil }

  deinit {
    camera.unregisterPreview(previewLayer)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    previewLayer.frame = bounds
    CATransaction.commit()
  }

  /// Only traces: the layer stays visible, a hidden layer is not drawn and might never start previewing.
  func applyPreviewing(_ previewing: Bool) {
    MediaTrace.emit("preview", previewing ? "preview shown" : "preview hidden")
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    sync(onScreen: window != nil)
  }

  /// Moves the view to another camera: the old one lets the preview go, the new one registers the layer and gets the demand when the view is on screen.
  func rebind(to next: MediaCamera) {
    guard next !== camera else { return }
    let wasAttached = attached
    sync(onScreen: false)
    camera.unregisterPreview(previewLayer)
    camera = next
    camera.registerPreview(previewLayer)
    sync(onScreen: wasAttached)
  }

  func isBound(to other: MediaCamera) -> Bool { other === camera }

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
    guard let view = view as? CameraPreviewLayerView else { return }
    view.zoomsByPinch = zoomsByPinch
    if view.isBound(to: camera) { return }
    view.rebind(to: camera)
    onLayer?(view.previewLayer)
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
    content(state).task(id: ObjectIdentifier(camera)) {
      state = camera.state
      for await next in camera.states { state = next }
    }
  }
}
#endif

#if os(iOS)
import CoreGraphics
import Foundation
import MediaLayer
import Observation

/// What the user chose: `crop` is a rectangle in fractions 0 to 1 of the photo after `rotation` (origin top left),
/// `rotation` is the clockwise turn in degrees, 0, 90, 180 or 270. Give both to `MediaLayer.prepareImage(_:crop:rotation:)`.
public struct MediaCropResult: Sendable, Equatable {
  public let crop: CGRect
  public let rotation: Int
}

/// The shape of the guide over the photo. The result is a rectangle in both.
public enum MediaCropShape: Sendable {
  case rect
  case circle
}

/// The state of one crop. The app makes it, gives it to `MediaCropper` and calls `confirm()` or `cancel()` from its own buttons.
/// `aspect` fixes the proportions of the frame (the photo moves under a still frame), without it the frame is free (handles on the frame).
/// `onFinish` is called once: with the result after `confirm()`, with `nil` after `cancel()`. A command after that, or before the photo is ready, is ignored.
@MainActor
@Observable
public final class MediaCrop {
  public private(set) var isReady = false
  public private(set) var failure: MediaError?
  public private(set) var isFinished = false
  public private(set) var rotation = 0
  public private(set) var isGesturing = false

  let aspect: CropAspect?
  let allowsRotation: Bool
  private(set) var rect = CropFractions(x: 0, y: 0, width: 1, height: 1)
  private(set) var photo = PixelSize(width: 0, height: 0)
  private let initial: CropFractions?
  private let limits: CropLimits
  private let onFinish: @MainActor (MediaCropResult?) -> Void
  private var upright = PixelSize(width: 0, height: 0)
  private var view = CropView(width: 0, height: 0, inset: 0)

  public init(
    aspect: CGSize? = nil,
    initial: CGRect? = nil,
    allowsRotation: Bool = true,
    limits: CropLimits = defaultCropLimits(),
    onFinish: @escaping @MainActor (MediaCropResult?) -> Void
  ) {
    self.aspect = aspect.flatMap { size in
      let shape = CropAspect(width: UInt32(clamping: Int(size.width.rounded())), height: UInt32(clamping: Int(size.height.rounded())))
      return shape.width > 0 && shape.height > 0 ? shape : nil
    }
    self.initial = initial.map { CropFractions(x: Double($0.minX), y: Double($0.minY), width: Double($0.width), height: Double($0.height)) }
    self.allowsRotation = allowsRotation
    self.limits = limits
    self.onFinish = onFinish
  }

  var setup: CropSetup { CropSetup(photo: photo, aspect: aspect, limits: limits) }

  var layout: CropLayout? { isReady ? cropLayout(rect: rect, setup: setup, view: view) : nil }

  /// The photo is on screen: `size` is its pixel size after the EXIF turn.
  func ready(photo size: PixelSize) {
    guard !isFinished, size.width > 0, size.height > 0 else { return }
    upright = size
    photo = size
    rotation = 0
    rect = cropInitial(setup: setup, initial: initial)
    failure = nil
    isReady = true
  }

  func fail(_ error: MediaError) {
    guard !isFinished else { return }
    failure = error
    isReady = false
  }

  func place(in size: CGSize, inset: CGFloat) {
    view = CropView(width: Double(size.width), height: Double(size.height), inset: Double(inset))
  }

  func pan(dx: CGFloat, dy: CGFloat) {
    guard isReady, !isFinished else { return }
    rect = cropPanView(rect: rect, setup: setup, view: view, dx: Double(dx), dy: Double(dy))
  }

  func zoom(by factor: CGFloat, at point: CGPoint) {
    guard isReady, !isFinished else { return }
    rect = cropZoomView(rect: rect, setup: setup, view: view, factor: Double(factor), focalX: Double(point.x), focalY: Double(point.y))
  }

  func drag(_ handle: CropHandle, dx: CGFloat, dy: CGFloat) {
    guard isReady, !isFinished else { return }
    rect = cropDragView(rect: rect, setup: setup, view: view, handle: handle, dx: Double(dx), dy: Double(dy))
  }

  func handle(at point: CGPoint, slop: CGFloat) -> CropHandle? {
    guard aspect == nil, let frame = layout?.frame else { return nil }
    return cropHandleAt(frame: frame, x: Double(point.x), y: Double(point.y), slop: Double(slop))
  }

  func gesture(_ active: Bool) {
    if isGesturing != active { isGesturing = active }
  }

  /// Turns the photo a quarter to the left (counter clockwise). Ignored when `allowsRotation` is false.
  public func turnLeft() { turn(clockwise: false) }

  /// Turns the photo a quarter to the right (clockwise). Ignored when `allowsRotation` is false.
  public func turnRight() { turn(clockwise: true) }

  /// The first frame and no turn again.
  public func reset() {
    guard isReady, !isFinished else { return }
    photo = upright
    rotation = 0
    rect = cropInitial(setup: setup, initial: initial)
  }

  /// Finishes with the rectangle of the frame and the turn. `nil` when the photo is not ready or the crop is already finished.
  @discardableResult
  public func confirm() -> MediaCropResult? {
    guard isReady, !isFinished, let done = cropFinish(rect: rect, setup: setup) else { return nil }
    let result = MediaCropResult(crop: CGRect(x: done.x, y: done.y, width: done.width, height: done.height), rotation: rotation)
    isFinished = true
    onFinish(result)
    return result
  }

  /// Finishes without a result.
  public func cancel() {
    guard !isFinished else { return }
    isFinished = true
    onFinish(nil)
  }

  private func turn(clockwise: Bool) {
    guard allowsRotation, isReady, !isFinished else { return }
    rect = cropTurn(rect: rect, setup: setup, clockwise: clockwise)
    photo = PixelSize(width: photo.height, height: photo.width)
    rotation = (rotation + (clockwise ? 90 : 270)) % 360
  }
}
#endif

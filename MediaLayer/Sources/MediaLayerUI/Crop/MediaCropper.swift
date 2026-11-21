#if os(iOS)
import MediaLayer
import SwiftUI
import UIKit

/// The canvas of a crop: the photo, the dimmed area outside the frame, the frame and its guide, and the gestures.
/// A crop with proportions keeps the frame still and moves the photo (drag and pinch). A free crop moves the frame (drag a handle to resize, drag inside to move, pinch to resize).
/// The app owns the buttons and calls `crop.confirm()` and `crop.cancel()`. The source is a local file. The view does no work while the finger is up.
public struct MediaCropper: View {
  private let source: any MediaSource
  private let crop: MediaCrop
  private let shape: MediaCropShape
  private let inset: CGFloat
  @Environment(\.displayScale) private var displayScale
  @State private var image: UIImage?
  @State private var handle: CropHandle?
  @State private var panned = CGSize.zero
  @State private var pinched: CGFloat = 1

  public init(source: some MediaSource, crop: MediaCrop, shape: MediaCropShape = .rect, inset: CGFloat = 16) {
    self.source = source
    self.crop = crop
    self.shape = shape
    self.inset = inset
  }

  public var body: some View {
    GeometryReader { proxy in
      ZStack {
        Color.black
        if let image = image?.cgImage, let layout = crop.layout {
          photo(image, layout: layout)
          guide(layout.frame, size: proxy.size)
        }
      }
      .frame(width: proxy.size.width, height: proxy.size.height)
      .contentShape(Rectangle())
      .clipped()
      .gesture(drag.simultaneously(with: magnify))
      .onAppear { crop.place(in: proxy.size, inset: inset) }
      .onChange(of: proxy.size) { _, size in crop.place(in: size, inset: inset) }
      .task(id: source.url) {
        crop.place(in: proxy.size, inset: inset)
        let pixels = max(proxy.size.width, proxy.size.height) * displayScale
        let side = Int(min(4096, max(pixels, 256)))
        let sharpSide = Int(min(4096, max(pixels * 3, 256)))
        guard let first = await crop.load(source, maxSide: side) else { return }
        image = first
        if sharpSide > side, let sharp = await crop.load(source, maxSide: sharpSide, announce: false), !Task.isCancelled { image = sharp }
      }
    }
    .accessibilityIdentifier("media-cropper")
  }

  private func photo(_ image: CGImage, layout: CropLayout) -> some View {
    Image(uiImage: UIImage(cgImage: image, scale: 1, orientation: Self.orientation(crop.rotation)))
      .resizable()
      .frame(width: layout.photo.width, height: layout.photo.height)
      .position(x: layout.photo.x + layout.photo.width / 2, y: layout.photo.y + layout.photo.height / 2)
  }

  private func guide(_ frame: CropBox, size: CGSize) -> some View {
    let box = CGRect(x: frame.x, y: frame.y, width: frame.width, height: frame.height)
    return ZStack {
      Path { path in
        path.addRect(CGRect(origin: .zero, size: size))
        if shape == .circle { path.addEllipse(in: box) } else { path.addRect(box) }
      }
      .fill(Color.black.opacity(0.55), style: FillStyle(eoFill: true))
      if shape == .circle {
        Circle().stroke(Color.white, lineWidth: 1.5).frame(width: box.width, height: box.height).position(x: box.midX, y: box.midY)
      } else {
        Path { path in path.addRect(box) }.stroke(Color.white, lineWidth: 1.5)
      }
      if crop.isGesturing {
        Path { path in
          for step in thirds {
            path.move(to: CGPoint(x: box.minX + box.width * step, y: box.minY))
            path.addLine(to: CGPoint(x: box.minX + box.width * step, y: box.maxY))
            path.move(to: CGPoint(x: box.minX, y: box.minY + box.height * step))
            path.addLine(to: CGPoint(x: box.maxX, y: box.minY + box.height * step))
          }
        }
        .stroke(Color.white.opacity(0.6), lineWidth: 0.5)
      }
    }
    .allowsHitTesting(false)
  }

  private var thirds: [CGFloat] { [1.0 / 3.0, 2.0 / 3.0] }

  private var drag: some Gesture {
    DragGesture(minimumDistance: 0)
      .onChanged { value in
        if !crop.isGesturing {
          crop.gesture(true)
          handle = crop.handle(at: value.startLocation, slop: 24)
          panned = .zero
        }
        let step = CGSize(width: value.translation.width - panned.width, height: value.translation.height - panned.height)
        panned = value.translation
        if let handle { crop.drag(handle, dx: step.width, dy: step.height) } else { crop.pan(dx: step.width, dy: step.height) }
      }
      .onEnded { _ in
        crop.gesture(false)
        handle = nil
        panned = .zero
      }
  }

  private var magnify: some Gesture {
    MagnifyGesture()
      .onChanged { value in
        crop.gesture(true)
        crop.zoom(by: value.magnification / pinched, at: value.startLocation)
        pinched = value.magnification
      }
      .onEnded { _ in
        crop.gesture(false)
        pinched = 1
      }
  }

  static func orientation(_ rotation: Int) -> UIImage.Orientation {
    switch rotation {
    case 90: .right
    case 180: .down
    case 270: .left
    default: .up
    }
  }
}
#endif

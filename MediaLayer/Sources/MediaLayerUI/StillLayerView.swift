#if canImport(UIKit)
import SwiftUI
import UIKit

final class StillLayerHost: UIView {
  var shape = MediaShape.rectangle {
    didSet { if shape != oldValue { setNeedsLayout() } }
  }
  var placement = ImagePlacement() {
    didSet { if placement != oldValue { setNeedsLayout() } }
  }

  override init(frame: CGRect) {
    super.init(frame: frame)
    isOpaque = false
    backgroundColor = .clear
    isUserInteractionEnabled = false
    layer.masksToBounds = true
  }

  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = shape.radius(in: bounds.size)
    ImageAlign.apply(to: layer, image: placement.image, bounds: bounds.size, contentMode: placement.contentMode, alignment: placement.alignment)
  }
}

/// A still image drawn as the contents of the layer that carries the shape and the grayscale conversion.
struct StillLayerView: UIViewRepresentable {
  let image: UIImage
  let contentMode: ContentMode
  let grayscale: Bool
  let shape: MediaShape
  var alignment = Alignment.center
  var tint: CGColor?

  final class Coordinator {
    var source: UIImage?
    var grayscale = false
    var tint: CGColor?
  }

  func makeCoordinator() -> Coordinator { Coordinator() }

  func makeUIView(context: Context) -> StillLayerHost { StillLayerHost() }

  func updateUIView(_ view: StillLayerHost, context: Context) {
    view.shape = shape
    view.placement = ImagePlacement(image: image.size, contentMode: contentMode, alignment: alignment)
    let coordinator = context.coordinator
    guard coordinator.source !== image || coordinator.grayscale != grayscale || coordinator.tint != tint else { return }

    coordinator.source = image
    coordinator.grayscale = grayscale
    coordinator.tint = tint
    view.layer.contents = image.cgImage.map { AnimatedFramesView.displayed($0, grayscale: grayscale, tint: tint) }
    TestChange.fire()
  }

  func sizeThatFits(_ proposal: ProposedViewSize, uiView: StillLayerHost, context: Context) -> CGSize? {
    ImageAlign.size(for: proposal, image: image.size, contentMode: contentMode, alignment: alignment)
  }
}
#endif

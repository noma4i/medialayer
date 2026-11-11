#if canImport(UIKit)
import SwiftUI
import UIKit

final class StillLayerHost: UIView {
  var shape = MediaShape.rectangle {
    didSet { if shape != oldValue { setNeedsLayout() } }
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
  }
}

/// A still image drawn as the contents of the layer that carries the shape and the grayscale conversion.
struct StillLayerView: UIViewRepresentable {
  let image: UIImage
  let contentMode: ContentMode
  let grayscale: Bool
  let shape: MediaShape

  final class Coordinator {
    var source: UIImage?
    var grayscale = false
  }

  func makeCoordinator() -> Coordinator { Coordinator() }

  func makeUIView(context: Context) -> StillLayerHost { StillLayerHost() }

  func updateUIView(_ view: StillLayerHost, context: Context) {
    view.layer.contentsGravity = contentMode == .fill ? .resizeAspectFill : .resizeAspect
    view.shape = shape
    let coordinator = context.coordinator
    guard coordinator.source !== image || coordinator.grayscale != grayscale else { return }

    coordinator.source = image
    coordinator.grayscale = grayscale
    view.layer.contents = image.cgImage.map { grayscale ? GrayscaleImage.convert($0) : $0 }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, uiView: StillLayerHost, context: Context) -> CGSize? {
    proposal.replacingUnspecifiedDimensions()
  }
}
#endif

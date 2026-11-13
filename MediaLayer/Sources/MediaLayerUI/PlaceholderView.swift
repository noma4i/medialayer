import AVFoundation
import MediaLayer
import SwiftUI

enum PlaceholderContent {
  case blur(CGImage)
  case cover(PlatformImage)
  case firstFrame(CGImage)
  case none

  var label: String {
    switch self {
    case .blur: "blur"
    case .cover: "cover"
    case .firstFrame: "firstframe"
    case .none: "none"
    }
  }
}

struct PlaceholderView: View {
  let item: MediaItem
  let ready: Bool
  var enabled = true
  let context: MediaContext
  var decode: @Sendable (String) -> CGImage? = { PlaceholderPixels.blurHash($0)?.cgImage }
  var still: @Sendable (URL, CGSize, CGFloat) async -> CGImage? = { await PlaceholderStill.frame(of: $0, points: $1, scale: $2) }
  var published: @MainActor (PlaceholderContent) -> Void = { _ in }
  @State private var current: Current?
  @Environment(\.displayScale) private var displayScale

  var body: some View {
    GeometryReader { geometry in
      let source = Source(item: item, context: ObjectIdentifier(context))
      ZStack {
        if let current, current.source == source {
          switch current.content {
          case .blur(let image): Image(decorative: image, scale: 1).resizable().scaledToFill()
          case .cover(let image): Image(platform: image).resizable().scaledToFill()
          case .firstFrame(let image): Image(decorative: image, scale: 1).resizable().scaledToFill()
          case .none: EmptyView()
          }
        }
      }
      .frame(width: geometry.size.width, height: geometry.size.height)
      .task(id: PlanKey(source: source, ready: ready, enabled: enabled, points: geometry.size, scale: displayScale)) {
        guard enabled, let content = await resolve(points: geometry.size, scale: displayScale), !Task.isCancelled else { return }
        if current?.source != source || current?.content.label != content.label {
          MediaTrace.emit("view", "placeholder \(MediaTrace.tail(item.url)) \(content.label)")
        }
        current = Current(source: source, content: content)
        published(content)
      }
    }
  }

  private func resolve(points: CGSize, scale: CGFloat) async -> PlaceholderContent? {
    let plan = await context.places.placeholder(source: item, ready: ready)
    guard !Task.isCancelled else { return nil }
    switch plan {
    case .blurHash(let hash):
      let image = await Self.decodeOffMain(hash, decode)
      guard !Task.isCancelled else { return nil }
      return image.map { .blur($0) } ?? PlaceholderContent.none
    case .cover(let url):
      let image = await Self.loadCover(url, for: item, context: context, points: points, scale: scale)
      guard !Task.isCancelled else { return nil }
      return image.map { .cover($0) } ?? PlaceholderContent.none
    case .firstFrame:
      guard let url = URL(string: item.url), url.isFileURL else { return PlaceholderContent.none }
      let image = await still(url, points, scale)
      guard !Task.isCancelled else { return nil }
      return image.map { .firstFrame($0) } ?? PlaceholderContent.none
    case .none:
      return PlaceholderContent.none
    }
  }

  @concurrent
  static func decodeOffMain(_ hash: String, _ decode: @Sendable (String) -> CGImage?) async -> CGImage? {
    guard !Task.isCancelled else { return nil }
    return decode(hash)
  }

  private struct Current {
    let source: Source
    let content: PlaceholderContent
  }

  private struct Source: Hashable {
    let kind: Kind
    let url: String
    let headers: [String: String]
    let cover: String?
    let blurHash: String?
    let context: ObjectIdentifier

    init(item: MediaItem, context: ObjectIdentifier) {
      kind = item.kind
      url = item.url
      headers = item.headers
      cover = item.cover
      blurHash = item.blurHash
      self.context = context
    }
  }

  private struct PlanKey: Hashable {
    let source: Source
    let ready: Bool
    let enabled: Bool
    let points: CGSize
    let scale: CGFloat
  }

  static func loadCover(_ url: String, for item: MediaItem, context: MediaContext, points: CGSize, scale: CGFloat) async -> PlatformImage? {
    let source = CoverSource(url: url, headers: item.headers)
    guard let request = await MediaImagePipeline.prepareOffMain(places: context.places, for: source, points: points, scale: scale).request else { return nil }
    guard let image = try? await context.pipeline.image(for: request) else { return nil }

    return image
  }
}

private struct CoverSource: MediaSource {
  let url: String
  let headers: [String: String]
  var kind: Kind { .image }
}

enum PlaceholderStill {
  @concurrent
  static func frame(of url: URL, points: CGSize, scale: CGFloat) async -> CGImage? {
    guard !Task.isCancelled, points.width > 0, points.height > 0 else { return nil }
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
    generator.appliesPreferredTrackTransform = true
    generator.maximumSize = CGSize(width: points.width * scale, height: points.height * scale)
    return await withTaskCancellationHandler {
      try? await generator.image(at: .zero).image
    } onCancel: { generator.cancelAllCGImageGeneration() }
  }
}

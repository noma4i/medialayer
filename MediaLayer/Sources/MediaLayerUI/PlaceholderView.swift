import MediaLayer
import SwiftUI

struct PlaceholderView: View {
  let item: MediaItem
  let ready: Bool
  let context: MediaContext
  @State private var plan: Placeholder = .none
  @State private var blur: CGImage?
  @State private var cover: PlatformImage?
  @Environment(\.displayScale) private var displayScale

  var body: some View {
    GeometryReader { geometry in
      ZStack {
        if let blur { Image(decorative: blur, scale: 1).resizable().scaledToFill() }
        if let cover { Image(platform: cover).resizable().scaledToFill() }
      }
      .frame(width: geometry.size.width, height: geometry.size.height)
      .task(id: PlanKey(url: item.url, headers: item.headers, ready: ready, points: geometry.size, scale: displayScale)) {
        plan = await context.places.placeholder(source: item, ready: ready)
        blur = nil
        cover = nil
        switch plan {
        case .blurHash(let hash): blur = PlaceholderPixels.blurHash(hash)?.cgImage
        case .cover(let url): cover = await Self.loadCover(url, for: item, context: context, points: geometry.size, scale: displayScale)
        case .firstFrame, .none: break
        }
      }
    }
  }

  private struct PlanKey: Hashable {
    let url: String
    let headers: [String: String]
    let ready: Bool
    let points: CGSize
    let scale: CGFloat
  }

  static func loadCover(_ url: String, for item: MediaItem, context: MediaContext, points: CGSize, scale: CGFloat) async -> PlatformImage? {
    let source = CoverSource(url: url, headers: item.headers)
    guard let request = MediaImagePipeline.request(places: context.places, for: source, points: points, scale: scale, contentMode: .fill) else { return nil }
    guard let image = try? await context.pipeline.image(for: request) else { return nil }

    return image
  }
}

private struct CoverSource: MediaSource {
  let url: String
  let headers: [String: String]
  var kind: Kind { .image }
}

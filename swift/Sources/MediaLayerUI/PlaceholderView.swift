import MediaLayer
import Nuke
import SwiftUI

struct PlaceholderView: View {
  let item: MediaItem
  let ready: Bool
  let context: MediaContext
  @State private var plan: Placeholder = .none
  @State private var blur: CGImage?
  @State private var cover: PlatformImage?

  var body: some View {
    ZStack {
      if let blur { Image(decorative: blur, scale: 1).resizable().scaledToFill() }
      if let cover { Image(platform: cover).resizable().scaledToFill() }
    }
    .task(id: PlanKey(url: item.url, ready: ready)) {
      plan = await context.places.placeholder(source: item, ready: ready)
      blur = nil
      cover = nil
      switch plan {
      case .blurHash(let hash): blur = PlaceholderPixels.blurHash(hash)?.cgImage
      case .cover(let url): cover = await load(url)
      case .firstFrame, .none: break
      }
    }
  }

  private struct PlanKey: Hashable {
    let url: String
    let ready: Bool
  }

  private func load(_ url: String) async -> PlatformImage? {
    guard let address = URL(string: url) else { return nil }
    return try? await context.pipeline.image(for: ImageRequest(url: address))
  }
}

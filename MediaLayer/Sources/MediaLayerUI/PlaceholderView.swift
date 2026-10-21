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
    .task(id: PlanKey(url: item.url, headers: item.headers, ready: ready)) {
      plan = await context.places.placeholder(source: item, ready: ready)
      blur = nil
      cover = nil
      switch plan {
      case .blurHash(let hash): blur = PlaceholderPixels.blurHash(hash)?.cgImage
      case .cover(let url): cover = await Self.loadCover(url, for: item, context: context)
      case .firstFrame, .none: break
      }
    }
  }

  private struct PlanKey: Hashable {
    let url: String
    let headers: [String: String]
    let ready: Bool
  }

  static func loadCover(_ url: String, for item: MediaItem, context: MediaContext) async -> PlatformImage? {
    guard let address = URL(string: url) else { return nil }
    var urlRequest = URLRequest(url: address)
    for (name, value) in item.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    var request = ImageRequest(urlRequest: urlRequest)
    request.imageID = context.places.cacheKey(url: url, headers: item.headers)
    guard let image = try? await context.pipeline.image(for: request) else { return nil }

    ToneFrames.submit(image.toneImage, source: item, places: context.places)
    return image
  }
}

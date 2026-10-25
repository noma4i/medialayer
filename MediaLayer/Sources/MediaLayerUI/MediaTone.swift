import MediaLayer
import Observation
import SwiftUI

@MainActor
@Observable
final class ToneModel {
  private(set) var tone: MediaTone?
  private(set) var error: MediaError?
  private var generation = 0
  private var followed: MediaItem?

  /// A new source clears the value of the old one; a new band keeps it until the new value arrives.
  func begin(source: MediaItem?) -> Int {
    generation += 1
    if source != followed {
      tone = nil
      error = nil
    }
    followed = source
    return generation
  }

  func apply(_ update: ToneUpdate, generation: Int) {
    guard generation == self.generation else { return }
    switch update {
    case .tone(let value):
      tone = value
      error = nil
    case .failed(let failure):
      error = failure
    }
  }

  func follow(places: MediaPlaces?, source: MediaItem?, band: Float) async {
    let generation = begin(source: source)
    guard let places, let source else { return }
    let stream = await Task.detached { places.toneStream(source: source, band: band) }.value
    for await update in stream {
      apply(update, generation: generation)
    }
  }
}

private struct ToneKey: Hashable {
  let source: MediaItem?
  let band: Float
  let context: ObjectIdentifier?
}

/// Follows the tone of `source` at `band` while mounted: `content` gets the current tone (nil while unknown) and the failure of the watch.
public struct WithMediaTone<Content: View>: View {
  private let source: MediaItem?
  private let band: Float
  private let content: (MediaTone?, MediaError?) -> Content
  @ResolvedMediaContext private var context
  @State private var model = ToneModel()

  public init(source: (any MediaSource)?, band: Float, @ViewBuilder content: @escaping (MediaTone?, MediaError?) -> Content) {
    self.source = source?.item
    self.band = band
    self.content = content
  }

  public var body: some View {
    content(model.tone, model.error)
      .task(id: ToneKey(source: source, band: band, context: context.map { ObjectIdentifier($0) })) {
        await model.follow(places: context?.places, source: source, band: band)
      }
  }
}

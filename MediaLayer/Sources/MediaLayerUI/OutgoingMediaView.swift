import MediaLayer
import SwiftUI

/// What is drawn while the remote copy is not ready.
public enum OutgoingCover {
  case local
  case blurredLocal(radius: CGFloat)
  case custom(@MainActor () -> AnyView)
}

struct OutgoingStyle {
  var cover: OutgoingCover = .local
  var loader: (@MainActor (OutgoingPhase, Bool) -> AnyView)?
  var failure: (@MainActor (OutgoingFailure) -> AnyView)?
}

private struct OutgoingStyleKey: EnvironmentKey {
  static let defaultValue = OutgoingStyle()
}

extension EnvironmentValues {
  var outgoingStyle: OutgoingStyle {
    get { self[OutgoingStyleKey.self] }
    set { self[OutgoingStyleKey.self] = newValue }
  }
}

extension View {
  public func outgoingCover(_ cover: OutgoingCover) -> some View {
    transformEnvironment(\.outgoingStyle) { $0.cover = cover }
  }

  /// Drawn over the cover while the item is busy; `hasLocal` is false for an item with no local original, which draws no loader by default.
  public func outgoingLoader<Loader: View>(@ViewBuilder _ loader: @escaping @MainActor (_ phase: OutgoingPhase, _ hasLocal: Bool) -> Loader) -> some View {
    transformEnvironment(\.outgoingStyle) { $0.loader = { phase, hasLocal in AnyView(loader(phase, hasLocal)) } }
  }

  /// Drawn over the cover when the item failed; the reason tells the upload from the remote.
  public func outgoingFailure<Failure: View>(@ViewBuilder _ failure: @escaping @MainActor (_ reason: OutgoingFailure) -> Failure) -> some View {
    transformEnvironment(\.outgoingStyle) { $0.failure = { reason in AnyView(failure(reason)) } }
  }
}

/// Draws an outgoing item: the cover until the remote copy is ready, then the remote picture in the same frame, with no animation.
/// A video item draws its cover only. The phase is the accessibility value and the identifier is `outgoing.<key>`.
public struct OutgoingMediaView: View {
  private let item: OutgoingMedia
  private let contentMode: ContentMode
  @Environment(\.outgoingStyle) private var style

  public init(_ item: OutgoingMedia, contentMode: ContentMode = .fill) {
    self.item = item
    self.contentMode = contentMode
  }

  public var body: some View {
    let entry = item.entry
    let phase = entry.phase
    ZStack {
      picture(entry, phase: phase)
      if entry.view.showsLoader, let loader = style.loader { loader(phase, entry.state.inputs.hasLocal) }
      if case .failed(let reason) = phase, let failure = style.failure { failure(reason) }
    }
    .clipped()
    .accessibilityElement(children: .contain)
    .accessibilityValue(phase.name)
    .accessibilityIdentifier("outgoing.\(entry.key)")
    .onAppear { entry.appeared() }
    .onDisappear { entry.disappeared() }
  }

  @ViewBuilder
  private func picture(_ entry: OutgoingEntry, phase: OutgoingPhase) -> some View {
    if phase == .ready, entry.kind == .image, let remote = entry.remote {
      MediaImage(
        MediaSourceValue(url: remote, kind: .image),
        preview: entry.localURL.map { MediaSourceValue.file($0, kind: .image) },
        contentMode: contentMode
      ) { state in
        if state == .ready || state == .rendered {
          Color.clear.onAppear { entry.swapDrawn() }
        } else {
          Color.clear
        }
      }
    } else {
      cover(entry)
    }
  }

  @ViewBuilder
  private func cover(_ entry: OutgoingEntry) -> some View {
    switch style.cover {
    case .custom(let make):
      make()
    case .local:
      local(entry)
    case .blurredLocal(let radius):
      local(entry).blur(radius: radius).clipped()
    }
  }

  @ViewBuilder
  private func local(_ entry: OutgoingEntry) -> some View {
    if let url = entry.localURL {
      MediaImage(MediaSourceValue.file(url, kind: .image), contentMode: contentMode)
    } else if let hash = entry.blurHash, let image = PlaceholderPixels.blurHash(hash)?.cgImage {
      Image(decorative: image, scale: 1).resizable().aspectRatio(contentMode: contentMode)
    } else {
      Color.clear
    }
  }
}

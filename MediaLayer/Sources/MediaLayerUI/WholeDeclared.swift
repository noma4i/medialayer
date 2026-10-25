import MediaLayer
import SwiftUI

@MainActor
public final class MediaWholeHold {
  private var declaration: MediaWholeDeclaration?
  private var layer: ObjectIdentifier?
  private var held = Set<MediaItem>()

  public init() {}

  public func declare(_ items: [MediaItem], in media: MediaLayer?) {
    guard let media else { return release() }
    let identity = ObjectIdentifier(media)
    if layer != identity { release() }
    do {
      let open = try declaration ?? media.openWholeDeclaration()
      declaration = open
      layer = identity
      let next = Set(items)
      let fresh = items.filter { !held.contains($0) }
      let gone = held.subtracting(next)
      if !gone.isEmpty { try open.remove(items: Array(gone)) }
      if !fresh.isEmpty { try open.add(items: fresh) }
      held = next
      Media.clearFailure()
    } catch {
      Media.recordOpenFailure(error)
    }
  }

  public func add(_ items: [MediaItem], in media: MediaLayer?) {
    declare(Array(held) + items, in: media)
  }

  public func release() {
    declaration?.release()
    declaration = nil
    layer = nil
    held = []
  }
}

private struct WholeDeclaredModifier: ViewModifier {
  @ResolvedMediaContext private var context
  @State private var hold = MediaWholeHold()
  let items: [MediaItem]

  private var layer: MediaLayer? { context?.places as? MediaLayer }

  func body(content: Content) -> some View {
    content
      .onAppear { hold.declare(items, in: layer) }
      .onChange(of: items) { _, next in hold.declare(next, in: layer) }
      .onDisappear { hold.release() }
  }
}

extension View {
  public func mediaWholeDeclared(_ items: [MediaItem]) -> some View {
    modifier(WholeDeclaredModifier(items: items))
  }
}

import MediaLayer
import SwiftUI

struct SuspensionIntent: Equatable {
  var layer: ObjectIdentifier?
  var active: Bool
  var groups: [String]
}

@MainActor
final class SuspensionHold {
  private var held: PlaybackSuspension?
  private var applied: SuspensionIntent?

  func apply(_ intent: SuspensionIntent, layer: MediaLayer?) {
    guard intent.active, let layer else { return release() }
    if held != nil, applied == intent { return }
    do {
      let next = try layer.suspendPlayback(allowGroups: intent.groups)
      held?.release()
      held = next
      applied = intent
      Media.clearFailure()
    } catch {
      applied = nil
      Media.recordFailure(error)
    }
  }

  func release() {
    held?.release()
    held = nil
    applied = nil
  }
}

private struct PlaybackSuspendedModifier: ViewModifier {
  @ResolvedMediaContext private var context
  @State private var hold = SuspensionHold()
  let active: Bool
  let allowGroups: [String]

  private var layer: MediaLayer? { context?.places as? MediaLayer }
  private var intent: SuspensionIntent {
    SuspensionIntent(layer: layer.map(ObjectIdentifier.init), active: active, groups: allowGroups)
  }

  func body(content: Content) -> some View {
    content
      .onAppear { hold.apply(intent, layer: layer) }
      .onChange(of: intent) { _, next in hold.apply(next, layer: layer) }
      .onDisappear { hold.release() }
  }
}

extension View {
  public func mediaPlaybackSuspended(active: Bool = true, allowGroups: [String] = []) -> some View {
    modifier(PlaybackSuspendedModifier(active: active, allowGroups: allowGroups))
  }
}

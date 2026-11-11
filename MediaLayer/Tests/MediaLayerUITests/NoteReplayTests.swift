#if canImport(UIKit)
import Foundation
import MediaLayer
import MediaLayerTesting
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct NoteSource: MediaSource {
  let url: String
  let kind: Kind
  var headers: [String: String] { [:] }
}

private final class NoteSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var events: [String] = []
  private var loaded: UInt64 = 0

  var generation: UInt64 { lock.withLock { loaded } }
  var all: [String] { lock.withLock { events } }
  func count(_ name: String) -> Int { all.filter { $0 == name }.count }

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    lock.withLock {
      switch command {
      case let .load(_, _, generation): loaded = generation; events.append("load")
      case let .seek(positionMs): events.append("seek \(positionMs)")
      case .unload: events.append("unload")
      case .play: events.append("play")
      default: break
      }
    }
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 60_000, durationMs: 60_000, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class NoteSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private struct NoteCell: View {
  let context: MediaContext

  var body: some View {
    MediaAudio(NoteSource(url: "file:///note.m4a", kind: .audio), playing: true) { _ in Color.clear }
      .environment(\.mediaContext, context)
      .frame(width: 80, height: 80)
  }
}

private func settles(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct NoteReplayTests {
    @Test
    func inv_ingress_44_aNoteThatEndedAndWasUnmountedStartsFromTheStartWhenMountedAgain() async throws {
      let slot = NoteSlot()
      let media = try MediaLayer.open(config: OpenConfig(
        dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
        fetcher: TestFetcher(), settings: NoteSettings(), slots: slot,
        device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
        defaults: defaultLimits(), pool: defaultPoolSettings()
      ))
      let context = MediaContext(places: media)
      let first = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      first.rootViewController = UIHostingController(rootView: NoteCell(context: context))
      first.isHidden = false
      #expect(await settles { slot.count("load") == 1 && slot.count("play") == 1 })
      media.playerEvents().ready(slot: 0, generation: slot.generation)
      media.playerEvents().ended(slot: 0, generation: slot.generation)
      #expect(await settles { slot.count("seek 0") >= 1 })

      first.isHidden = true
      first.rootViewController = nil
      #expect(await settles { slot.count("unload") == 1 })
      let second = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      second.rootViewController = UIHostingController(rootView: NoteCell(context: context))
      second.isHidden = false
      #expect(await settles { slot.count("load") == 2 && slot.count("play") == 2 })
      defer {
        media.releaseOwned()
        second.isHidden = true
        second.rootViewController = nil
      }

      let events = slot.all
      let load = try #require(events.lastIndex(of: "load"))
      #expect(!events[load...].contains { $0.hasPrefix("seek ") && $0 != "seek 0" }, "\(events)")
    }
  }
}
#endif

#if canImport(UIKit)
import Foundation
import MediaLayer
import MediaLayerTesting
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private let pageCount = 50

private struct PageSource: MediaSource {
  let url: String
  var kind: Kind { .video }
  var headers: [String: String] { [:] }
}

private final class CountingSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var created = 0
  private var played: Set<String> = []
  var createdCount: Int { lock.withLock { created } }
  var hasPlayed: Bool { lock.withLock { !played.isEmpty } }

  func create(slot: UInt32) async -> CreatedPlayer {
    lock.withLock { created += 1 }
    return CreatedPlayer(player: self)
  }
  func apply(command: SlotCall) async {
    if case .play = command { lock.withLock { played.insert("play") } }
  }
  func dispose() async {}
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

private struct Pages: View {
  let context: MediaContext

  var body: some View {
    ScrollView(.horizontal) {
      HStack(spacing: 0) {
        ForEach(0..<pageCount, id: \.self) { index in
          MediaVideo(PageSource(url: "file:///page-\(index).mp4")).frame(width: 200, height: 200)
        }
      }
    }
    .environment(\.mediaContext, context)
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MountedVideoPagesTests {
    @Test
    func inv_mount_01_manyMountedVideoPagesCreateOnlyAsManyPlayersAsThereAreVisiblePages() async throws {
      let slot = CountingSlot()
      let media = try MediaLayer.open(config: OpenConfig(
        dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
        availableBytes: UInt64.max, fetcher: TestFetcher(), settings: QuietSettings(), slots: slot,
        device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
        defaults: defaultLimits(), pool: defaultPoolSettings()
      ))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      window.rootViewController = UIHostingController(rootView: Pages(context: MediaContext(places: media)))
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }

      var settled = false
      for _ in 0..<400 where !settled {
        settled = slot.hasPlayed
        try await Task.sleep(for: .milliseconds(25))
      }
      await media.awaitClosed()

      #expect(settled)
      #expect(slot.createdCount == 1, "created \(slot.createdCount) players for \(pageCount) mounted pages, 1 visible")
    }
  }
}
#endif

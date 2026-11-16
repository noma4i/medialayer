#if canImport(UIKit)
import Foundation
import MediaLayer
import MediaLayerTesting
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct ConnectClip: MediaSource {
  let url = "file:///connect.mp4"
  let kind = Kind.video
  var headers: [String: String] { [:] }
}

private final class ConnectSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var played = 0
  var plays: Int { lock.withLock { played } }

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    if case .play = command { lock.withLock { played += 1 } }
    TestSignal.shared.fire()
  }
  func dispose() async {}
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

private func openConnectCore(_ slot: ConnectSlot) throws -> MediaLayer {
  try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    fetcher: TestFetcher(), settings: QuietSettings(), slots: slot,
    device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
    defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: .exclusive
  ))
}

private final class ConnectSeen: @unchecked Sendable {
  private let lock = NSLock()
  private var places: MediaPlaces?
  private var layer: MediaLayer?
  var seenPlaces: MediaPlaces? { lock.withLock { places } }
  var seenLayer: MediaLayer? { lock.withLock { layer } }

  func record(places: MediaPlaces?, layer: MediaLayer?) {
    lock.withLock {
      self.places = places
      self.layer = layer
    }
    TestSignal.shared.fire()
  }
}

private struct ConnectProbe: View {
  @ResolvedMediaContext private var context
  @Environment(\.mediaLayer) private var layer
  let seen: ConnectSeen

  var body: some View {
    let _ = seen.record(places: context?.places, layer: layer)
    return Color.clear.frame(width: 20, height: 20)
  }
}

private struct ConnectClipCell: View {
  let seen: ConnectSeen

  var body: some View {
    ZStack {
      MediaVideo(ConnectClip())
      ConnectProbe(seen: seen)
    }
    .frame(width: 80, height: 80)
  }
}

@MainActor
private func connectHost<Content: View>(_ view: Content) -> UIWindow {
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
  window.rootViewController = UIHostingController(rootView: view)
  window.isHidden = false
  return window
}

@MainActor
private func connectRemove(_ window: UIWindow) {
  window.isHidden = true
  window.rootViewController = nil
}

private func connectDrainMain() async {
  await MainActor.run {}
  await MainActor.run {}
}

private func untilConnect(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct ConnectWithoutAppMachineTests {
    private func gatedOpening(_ result: @escaping @Sendable () throws -> MediaLayer) -> (Task<MediaLayer, Error>, AsyncStream<Void>.Continuation) {
      let (gate, release) = AsyncStream<Void>.makeStream()
      let task = Task<MediaLayer, Error> {
        for await _ in gate { break }
        return try result()
      }
      return (task, release)
    }

    @Test(arguments: [false, true])
    func inv_native_26_openWithoutFetcherAndSettingsUsesUrlSessionAndUserDefaults(withSpec: Bool) async throws {
      let server = try LocalHTTPServer()
      try await server.start()
      defer { server.stop() }
      server.serve(LocalHTTPServer.payload, at: "/default.bin")
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      UserDefaults.standard.removeObject(forKey: "medialayer.limits")
      defer { UserDefaults.standard.removeObject(forKey: "medialayer.limits") }

      let media = try await withSpec ? MediaLayer.open(spec: "{}", directory: directory) : MediaLayer.open(directory: directory)
      var limits = try await media.limits()
      limits.budget = 512 * 1024 * 1024
      try await media.setLimits(limits: limits)
      _ = try? await media.fetchImage(url: server.url("/default.bin"), headers: [:])

      #expect(UserDefaults.standard.string(forKey: "medialayer.limits") != nil)
      #expect(server.requestCount(of: "/default.bin") > 0)
    }

    @Test
    func inv_native_27_openingTaskDeclaresNoPlaceUntilTheLayerIsReadyThenMountsItself() async throws {
      let slot = ConnectSlot()
      let media = try openConnectCore(slot)
      let seen = ConnectSeen()
      let (opening, release) = gatedOpening { media }
      let window = connectHost(ConnectClipCell(seen: seen).mediaLayer(opening: opening))
      defer {
        media.releaseOwned()
        connectRemove(window)
      }

      await connectDrainMain()
      #expect(slot.plays == 0)
      #expect(seen.seenLayer == nil)

      release.yield()
      #expect(await untilConnect { slot.plays > 0 })
    }

    @Test
    func inv_native_28_openingFailureShowsInMediaFailureAndDrawsTheContent() async throws {
      let seen = ConnectSeen()
      let (opening, release) = gatedOpening { throw MediaError.Fetch(detail: "connect boom") }
      try Media.setMuted(false)
      let window = connectHost(ConnectProbe(seen: seen).mediaLayer(opening: opening))
      defer { connectRemove(window) }

      release.yield()
      #expect(await untilConnect { Media.failure != nil })
      #expect(String(describing: Media.failure).contains("connect boom"))
      #expect(seen.seenLayer == nil)
      try Media.setMuted(false)
    }

    @Test
    func inv_native_29_viewOutsideTheProviderTakesTheProcessContextAndTheEnvironmentWinsAndRemovalClearsIt() async throws {
      let first = RecordingPlaces(), second = RecordingPlaces()
      let outside = ConnectSeen(), insideFirst = ConnectSeen()
      let firstWindow = connectHost(ConnectProbe(seen: insideFirst).mediaLayer(first))
      #expect(await untilConnect { ProcessMediaContext.shared.current?.places === first })
      let secondWindow = connectHost(Color.clear.mediaLayer(second))
      #expect(await untilConnect { ProcessMediaContext.shared.current?.places === second })
      let outsideWindow = connectHost(ConnectProbe(seen: outside))
      defer {
        connectRemove(firstWindow)
        connectRemove(secondWindow)
        connectRemove(outsideWindow)
      }

      #expect(await untilConnect { outside.seenPlaces === second })
      #expect(insideFirst.seenPlaces === first)

      connectRemove(firstWindow)
      await connectDrainMain()
      #expect(outside.seenPlaces === second)

      connectRemove(secondWindow)
      #expect(await untilConnect { outside.seenPlaces == nil })
    }

    @Test
    func inv_native_30_layerAccessIsNilBeforeReadyAndTheLayerAfterAndSeesTheProcessContext() async throws {
      let slot = ConnectSlot()
      let media = try openConnectCore(slot)
      let seen = ConnectSeen(), outside = ConnectSeen()
      let (opening, release) = gatedOpening { media }
      let window = connectHost(ConnectProbe(seen: seen).mediaLayer(opening: opening))
      let outsideWindow = connectHost(ConnectProbe(seen: outside))
      defer {
        connectRemove(window)
        connectRemove(outsideWindow)
        media.releaseOwned()
      }

      await connectDrainMain()
      #expect(seen.seenLayer == nil)
      #expect(outside.seenLayer == nil)

      release.yield()
      #expect(await untilConnect { seen.seenLayer === media })
      #expect(await untilConnect { outside.seenLayer === media })
    }
  }
}
#endif

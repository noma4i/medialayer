import Foundation
import MediaLayer
import MediaLayerTesting
import Observation
import SwiftUI
import Testing
@testable import MediaLayerUI

#if canImport(UIKit)
import UIKit

private final class SuspendProbeSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var entries: [String] = []
  private var unloadCount = 0
  var transport: [String] { lock.withLock { entries } }
  var unloads: Int { lock.withLock { unloadCount } }
  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case .play: lock.withLock { entries.append("play") }
    case .pause: lock.withLock { entries.append("pause") }
    case .unload: lock.withLock { unloadCount += 1 }
    default: break
    }
  }
  func dispose() async {}
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

private final class SuspendProbeSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

@Observable
private final class SuspendProbeModel {
  var active = false
  var groups: [String] = []
  var mounted = true
  var unrelated = 0
  var tab = 0
}

private struct SuspendProbeCell: View {
  let model: SuspendProbeModel
  let context: MediaContext

  var body: some View {
    let _ = model.unrelated
    Group {
      if model.mounted {
        TabView(selection: Binding(get: { model.tab }, set: { model.tab = $0 })) {
          Color.clear.mediaPlaybackSuspended(active: model.active, allowGroups: model.groups).tag(0)
          Color.clear.tag(1)
        }
      }
    }
    .environment(\.mediaContext, context)
  }
}

@Observable
private final class TabLifecycleProbe {
  var tab = 0
  var appeared = 0
  var disappeared = 0
}

private struct TabLifecycleCell: View {
  let probe: TabLifecycleProbe

  var body: some View {
    TabView(selection: Binding(get: { probe.tab }, set: { probe.tab = $0 })) {
      Color.clear.onAppear { probe.appeared += 1 }.onDisappear { probe.disappeared += 1 }.tag(0)
      Color.clear.tag(1)
    }
  }
}

private let probeClip = "file:///clip.mp4"

@MainActor
private func waits(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

@MainActor
private struct SuspendRig {
  let media: MediaLayer
  let slot = SuspendProbeSlot()
  let model = SuspendProbeModel()
  let window: UIWindow
  let host: UIHostingController<SuspendProbeCell>

  init(ingress: IngressLimits? = nil, mountsNow: Bool = true) async throws {
    let slot = self.slot
    media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: SuspendProbeSettings(), slots: slot,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), ingress: ingress
    ))
    media.replaceWindow(active: 0, sources: [MediaItem(kind: .video, url: probeClip, headers: [:], cover: nil, blurHash: nil, strategy: .window)])
    media.setPlaying(key: probeClip, playing: true, groups: [nil])
    _ = await waits { slot.transport == ["play"] }
    window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    host = UIHostingController(rootView: SuspendProbeCell(model: model, context: MediaContext(places: media)))
    window.rootViewController = host
    window.isHidden = false
  }

  func tearDown() {
    window.isHidden = true
    window.rootViewController = nil
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct PlaybackSuspendedTests {
    @Test
    func inv_session_18_aMountedSuspensionFollowsActiveAndNeverDuplicates() async throws {
      let rig = try await SuspendRig()
      defer { rig.tearDown() }

      rig.model.groups = ["other"]
      rig.model.unrelated += 1
      #expect(rig.slot.transport == ["play"])

      rig.model.active = true
      #expect(await waits { rig.slot.transport == ["play", "pause"] })
      rig.model.active = true
      rig.model.unrelated += 1
      rig.model.groups = ["other"]

      rig.model.active = false
      #expect(await waits { rig.slot.transport == ["play", "pause", "play"] })
      withExtendedLifetime(rig.media) {}
    }

    @Test
    func inv_session_18_aRepeatedIntentTakesNoSecondHold() async throws {
      let rig = try await SuspendRig(ingress: IngressLimits(entries: 256, owners: 3, bytes: 1 << 20))
      defer { rig.tearDown() }
      let hold = SuspensionHold()
      let intent = SuspensionIntent(layer: ObjectIdentifier(rig.media), active: true, groups: ["other"])

      hold.apply(intent, layer: rig.media)
      #expect(await waits { rig.slot.transport == ["play", "pause"] })
      hold.apply(intent, layer: rig.media)
      #expect(Media.failure == nil)
      hold.release()
      #expect(await waits { rig.slot.transport == ["play", "pause", "play"] })
    }

    @Test
    func inv_session_18_aSuspensionLeftBeforeTheLayerOpensHoldsNothing() async throws {
      let rig = try await SuspendRig()
      defer { rig.tearDown() }
      rig.model.active = true
      #expect(await waits { rig.slot.transport == ["play", "pause"] })

      rig.model.tab = 1
      #expect(await waits { rig.slot.transport == ["play", "pause", "play"] })
      rig.model.tab = 0
      #expect(await waits { rig.slot.transport == ["play", "pause", "play", "pause"] })

      rig.model.mounted = false
      #expect(await waits { rig.slot.transport == ["play", "pause", "play", "pause", "play"] })

      let unopened = UIWindow(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
      let model = SuspendProbeModel()
      model.active = true
      unopened.rootViewController = UIHostingController(rootView: Color.clear.mediaPlaybackSuspended(active: model.active))
      unopened.isHidden = false
      await MainActor.run {}
      await MainActor.run {}
      unopened.isHidden = true
      unopened.rootViewController = nil
      #expect(rig.slot.transport == ["play", "pause", "play", "pause", "play"])
    }

    @Test
    func inv_session_18_aTabSwitchDeliversTheDisappearToPlainContentInTheHarnessWindow() async throws {
      let probe = TabLifecycleProbe()
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
      window.rootViewController = UIHostingController(rootView: TabLifecycleCell(probe: probe))
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }
      #expect(await waits { probe.appeared == 1 })

      probe.tab = 1
      #expect(await waits { probe.disappeared == 1 }, "appeared=\(probe.appeared) disappeared=\(probe.disappeared)")
    }

    @Test
    func inv_session_19_aGroupChangeReplacesTheHoldWithoutAGap() async throws {
      let rig = try await SuspendRig()
      defer { rig.tearDown() }
      rig.model.groups = ["one"]
      rig.model.active = true
      #expect(await waits { rig.slot.transport == ["play", "pause"] })

      rig.model.groups = ["two"]
      rig.model.unrelated += 1
      await MainActor.run {}
      rig.model.active = false
      #expect(await waits { rig.slot.transport.count >= 3 })
      rig.media.close()
      await rig.media.awaitClosed()
      #expect(rig.slot.transport == ["play", "pause", "play"])
      withExtendedLifetime(rig.media) {}
    }

    @Test
    func inv_session_19_aReleaseReachesTheCoreThroughAFullInput() async throws {
      let rig = try await SuspendRig(ingress: IngressLimits(entries: 256, owners: 3, bytes: 1 << 20))
      defer { rig.tearDown() }
      rig.model.groups = ["one"]
      rig.model.active = true
      #expect(await waits { rig.slot.transport.last == "pause" })

      rig.model.groups = ["two"]
      #expect(await waits { Media.failure != nil })
      #expect(rig.slot.transport == ["play", "pause"])

      rig.model.active = false
      #expect(await waits { rig.slot.transport == ["play", "pause", "play"] })
      Media.clearFailure()
      withExtendedLifetime(rig.media) {}
    }

    @Test
    func inv_session_20_aPlayingPlaceSurvivesBackgroundAndForegroundWithoutAppCode() async throws {
      let rig = try await SuspendRig()
      defer { rig.tearDown() }
      let center = NotificationCenter()
      let names = SystemSignalNames(
        memoryWarning: Notification.Name("suspend.memory"), background: Notification.Name("suspend.background"),
        foreground: Notification.Name("suspend.foreground"), outputLost: Notification.Name("suspend.outputLost")
      )
      rig.media.followSystem(events: rig.media.playerEvents(), link: PlayerEventsLink(), center: center, names: names)

      center.post(name: names.background, object: nil)
      #expect(await waits { rig.slot.unloads >= 1 })
      center.post(name: names.foreground, object: nil)
      #expect(await waits { rig.slot.transport.count >= 2 && rig.slot.transport.last == "play" })
      rig.media.close()
      await rig.media.awaitClosed()
    }
  }
}
#endif

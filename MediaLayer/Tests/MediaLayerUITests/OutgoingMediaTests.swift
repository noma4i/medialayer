#if canImport(UIKit)
import MediaLayer
import MediaLayerTesting
import Nuke
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private func png(left: UIColor, right: UIColor) -> Data {
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  return UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32), format: format).pngData { context in
    left.setFill()
    context.fill(CGRect(x: 0, y: 0, width: 16, height: 32))
    right.setFill()
    context.fill(CGRect(x: 16, y: 0, width: 16, height: 32))
  }
}

@MainActor
private func pixel(of host: UIView, x: CGFloat) -> [UInt8] {
  var pixel = [UInt8](repeating: 0, count: 4)
  let context = CGContext(
    data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  )!
  context.translateBy(x: -x, y: -host.bounds.midY)
  host.layer.render(in: context)
  return pixel
}

@MainActor
private func until(_ condition: String, _ holds: () -> Bool) async {
  let end = Date().addingTimeInterval(5)
  while !holds() {
    guard Date() < end else {
      Issue.record("never became true within 5s: \(condition)")
      return
    }
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private struct Rig {
  let places = RecordingPlaces()
  let context: MediaContext
  let remote = FakeOutgoingRemote()
  let clock = ManualOutgoingClock()
  let store: OutgoingStore
  let directory: URL
  let local: URL

  init() {
    ImageMemory.install(ImageCache(), on: places)
    context = MediaContext(places: places)
    directory = FileManager.default.temporaryDirectory.appendingPathComponent("outgoing-\(UUID().uuidString)", isDirectory: true)
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    store = OutgoingStore(directory: directory.appendingPathComponent("store"))
    local = directory.appendingPathComponent("photo.png")
    try? png(left: .red, right: .red).write(to: local)
    places.answer(local.absoluteString, with: png(left: .red, right: .red))
    OutgoingRuntime.shared.forgetAll()
    OutgoingRuntime.shared.use(remote: remote, clock: clock, store: store)
  }

  func remoteURL(_ name: String = UUID().uuidString) -> String {
    let url = "https://o.test/\(name).png"
    places.answer(url, with: png(left: .blue, right: .blue))
    return url
  }

  func show(_ item: OutgoingMedia, cover: OutgoingCover = .local) -> (UIHostingController<AnyView>, UIWindow) {
    let view = OutgoingMediaView(item).outgoingCover(cover).environment(\.mediaContext, context).frame(width: 32, height: 32)
    let host = UIHostingController(rootView: AnyView(view))
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 32, height: 32))
    window.rootViewController = host
    window.isHidden = false
    host.view.backgroundColor = .clear
    host.safeAreaRegions = []
    host.view.frame = window.bounds
    return (host, window)
  }

  func finish() {
    OutgoingRuntime.shared.forgetAll()
    try? FileManager.default.removeItem(at: directory)
  }
}

@MainActor
private func identifiers(in object: Any?) -> [String] {
  guard let object else { return [] }
  var found: [String] = []
  if let node = object as? NSObject, node.responds(to: Selector(("accessibilityIdentifier"))), let id = node.value(forKey: "accessibilityIdentifier") as? String, !id.isEmpty { found.append(id) }
  if let view = object as? UIView {
    for element in view.accessibilityElements ?? [] { found += identifiers(in: element) }
    for sub in view.subviews { found += identifiers(in: sub) }
  }
  if let element = object as? NSObject, !(object is UIView) {
    let count = element.accessibilityElementCount()
    if count != NSNotFound { for index in 0..<count { found += identifiers(in: element.accessibilityElement(at: index)) } }
    for child in (element.value(forKey: "accessibilityElements") as? [Any]) ?? [] { found += identifiers(in: child) }
  }
  return found
}

private func isRed(_ pixel: [UInt8]) -> Bool { pixel[0] > 200 && pixel[2] < 60 }
private func isBlue(_ pixel: [UInt8]) -> Bool { pixel[2] > 200 && pixel[0] < 60 }

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(3)))
  struct OutgoingMediaTests {
    @Test
    func inv_outgoing_12_no_frame_without_a_picture_from_sending_to_ready_with_a_remote_that_answers_late() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = rig.remoteURL()
      rig.remote.hold(url)
      let item = OutgoingMedia(key: "k12", kind: .image, local: rig.local)
      let (host, window) = rig.show(item)
      defer { window.isHidden = true }
      await until("the local is drawn") { isRed(pixel(of: host.view, x: 16)) }

      item.forward(upload: .sending(progress: 0.5))
      item.remote = url
      var blank = 0
      for turn in 0..<60 {
        if turn == 30 { rig.remote.complete(url) }
        RunLoop.main.run(until: Date().addingTimeInterval(0.005))
        await Task.yield()
        let sample = pixel(of: host.view, x: 16)
        if !(isRed(sample) || isBlue(sample)) { blank += 1 }
      }
      await until("the item is ready") { item.phase == .ready }
      await until("the remote is drawn") { isBlue(pixel(of: host.view, x: 16)) }
      #expect(blank == 0)
    }

    @Test
    func inv_outgoing_13_the_blurred_local_is_blurred_only_before_ready_and_the_remote_is_sharp() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = "https://o.test/sharp-\(UUID().uuidString).png"
      rig.places.answer(url, with: png(left: .blue, right: .blue))
      let split = rig.directory.appendingPathComponent("split.png")
      try? png(left: .red, right: .blue).write(to: split)
      rig.places.answer(split.absoluteString, with: png(left: .red, right: .blue))
      rig.remote.hold(url)
      let item = OutgoingMedia(key: "k13", kind: .image, local: split)
      let (host, window) = rig.show(item, cover: .blurredLocal(radius: 8))
      defer { window.isHidden = true }
      await until("the blurred cover is drawn from the local") {
        return isRed(pixel(of: host.view, x: 4)) && isBlue(pixel(of: host.view, x: 28)) && identifiers(in: host.view).contains("outgoing.cover.blurred")
      }
      #expect(!identifiers(in: host.view).contains("outgoing.cover.local"))

      item.remote = url
      rig.remote.complete(url)
      await until("the item is ready") { item.phase == .ready }
      await until("the sharp remote replaces it") { isBlue(pixel(of: host.view, x: 4)) && isBlue(pixel(of: host.view, x: 15)) }
      await until("the cover is gone") { !identifiers(in: host.view).contains { $0.hasPrefix("outgoing.cover.") } }
    }

    @Test
    func inv_outgoing_14_a_remote_failure_keeps_the_local_and_retry_reaches_ready() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = rig.remoteURL()
      rig.remote.answer(url, .failed(status: 500, detail: "server"))
      let item = OutgoingMedia(key: "k14", kind: .image, local: rig.local)
      let (host, window) = rig.show(item)
      defer { window.isHidden = true }

      item.remote = url
      await until("the remote failed") { if case .failed(.remote) = item.phase { true } else { false } }
      #expect(isRed(pixel(of: host.view, x: 16)))
      #expect(rig.remote.requestCount(for: url) == 1)

      rig.remote.answer(url, .complete)
      item.retry()
      await until("the item is ready") { item.phase == .ready }
      await until("the remote is drawn") { isBlue(pixel(of: host.view, x: 16)) }
      #expect(rig.remote.requestCount(for: url) == 2)
    }

    @Test
    func inv_outgoing_15_can_open_is_false_before_ready_true_at_ready_and_the_app_rule_overrides() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = rig.remoteURL()
      rig.remote.hold(url)
      let item = OutgoingMedia(key: "k15", kind: .image, local: rig.local)
      #expect(!item.canOpen)
      item.forward(upload: .sending(progress: 0.3))
      #expect(!item.canOpen)
      item.remote = url
      #expect(!item.canOpen)
      item.openable = { _ in true }
      #expect(item.canOpen)
      item.openable = nil
      rig.remote.complete(url)
      await until("the item is ready") { item.phase == .ready }
      #expect(item.canOpen)
      item.openable = { _ in false }
      #expect(!item.canOpen)
    }

    @Test
    func inv_outgoing_16_at_most_two_remote_fetches_run_at_once_for_ten_items() async {
      let rig = Rig()
      defer { rig.finish() }
      var items: [OutgoingMedia] = []
      var urls: [String] = []
      for index in 0..<10 {
        let url = rig.remoteURL("album-\(index)-\(UUID().uuidString)")
        rig.remote.hold(url)
        let item = OutgoingMedia(key: "k16-\(index)", kind: .image, local: rig.local)
        item.remote = url
        items.append(item)
        urls.append(url)
      }
      await until("two fetches run") { rig.remote.inFlight == 2 }
      #expect(rig.remote.peakInFlight == 2)
      for url in urls {
        rig.remote.complete(url)
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        await Task.yield()
      }
      await until("all ten are ready") { items.allSatisfy { $0.phase == .ready } }
      #expect(rig.remote.peakInFlight == 2)
    }

    @Test
    func inv_outgoing_17_custody_survives_a_new_item_of_the_same_key_and_is_released_at_ready() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = rig.remoteURL()
      rig.remote.hold(url)
      let first = OutgoingMedia(key: "k17", kind: .image, local: rig.local)
      let kept = first.local
      #expect(kept != nil)
      #expect(rig.store.stored(for: "k17") != nil)

      OutgoingRuntime.shared.forgetAll()
      try? FileManager.default.removeItem(at: rig.local)
      let second = OutgoingMedia(key: "k17", kind: .image)
      #expect(second.hasLocal)
      #expect(second.local.map { FileManager.default.fileExists(atPath: $0.path) } == true)

      second.remote = url
      rig.remote.complete(url)
      await until("the item is ready") { second.phase == .ready }
      #expect(rig.store.stored(for: "k17") == nil)
      #expect(Media.outgoing(key: "k17")?.phase == .ready)
    }

    @Test
    func inv_outgoing_18_a_not_ready_remote_is_asked_again_only_while_on_screen() async {
      let rig = Rig()
      defer { rig.finish() }
      let url = rig.remoteURL()
      rig.remote.answer(url, .notReady)
      let item = OutgoingMedia(key: "k18", kind: .image, local: rig.local)
      item.remote = url
      await until("the remote answered not ready") { rig.remote.requestCount(for: url) == 1 }
      RunLoop.main.run(until: Date().addingTimeInterval(0.1))
      #expect(rig.clock.sleeping == 0)

      let (_, window) = rig.show(item)
      defer { window.isHidden = true }
      await until("the re-ask waits") { rig.clock.sleeping == 1 }
      #expect(rig.clock.requestedMilliseconds == [5000])

      rig.remote.answer(url, .complete)
      rig.clock.advance()
      await until("the item is ready") { item.phase == .ready }
      #expect(rig.remote.requestCount(for: url) == 2)
    }
  }
}
#endif

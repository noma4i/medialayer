#if canImport(UIKit)
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private let red = "00TI:j"
private let blue = "000036"
private let green = "0009dg"

private enum Tint: Equatable, Sendable { case red, blue, green, clear, other }

private enum Event: Equatable, Sendable {
  case ask(places: String, url: String, input: String?, ready: Bool, held: Bool)
  case returned(places: String, url: String)
  case decode(hash: String, onMain: Bool)
  case fetch(url: String)
  case published(Shown)
}

private enum Shown: Equatable, Sendable { case blur(Tint), cover(Tint), none }

private final class Events: @unchecked Sendable {
  private final class Waiter {
    let match: (Event) -> Bool
    let needed: Int
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Bool, Never>?
    init(match: @escaping (Event) -> Bool, needed: Int, continuation: CheckedContinuation<Bool, Never>) {
      self.match = match
      self.needed = needed
      self.continuation = continuation
    }
    func finish(_ value: Bool) {
      let pending = lock.withLock { () -> CheckedContinuation<Bool, Never>? in
        defer { continuation = nil }
        return continuation
      }
      pending?.resume(returning: value)
    }
  }

  private let lock = NSLock()
  private var items: [Event] = []
  private var waiters: [Waiter] = []

  var all: [Event] { lock.withLock { items } }

  func add(_ event: Event) {
    let ready = lock.withLock { () -> [Waiter] in
      items.append(event)
      let done = waiters.filter { waiter in items.filter(waiter.match).count >= waiter.needed }
      waiters.removeAll { waiter in done.contains { $0 === waiter } }
      return done
    }
    ready.forEach { $0.finish(true) }
  }

  func wait(atLeast needed: Int = 1, bound: Double = 15, _ match: @escaping (Event) -> Bool) async -> Bool {
    await withCheckedContinuation { continuation in
      let waiter = Waiter(match: match, needed: needed, continuation: continuation)
      let satisfied = lock.withLock { () -> Bool in
        if items.filter(match).count >= needed { return true }
        waiters.append(waiter)
        return false
      }
      if satisfied { waiter.finish(true); return }
      DispatchQueue.global().asyncAfter(deadline: .now() + bound) { waiter.finish(false) }
    }
  }

  func count(_ match: (Event) -> Bool) -> Int { all.filter(match).count }
}

private final class Gate: @unchecked Sendable {
  private let lock = NSLock()
  private var opened = false
  private var waiters: [CheckedContinuation<Void, Never>] = []
  func wait() async {
    await withCheckedContinuation { continuation in
      let pass = lock.withLock { () -> Bool in
        if opened { return true }
        waiters.append(continuation)
        return false
      }
      if pass { continuation.resume() }
    }
  }
  func open() {
    let pending = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      opened = true
      defer { waiters = [] }
      return waiters
    }
    pending.forEach { $0.resume() }
  }
}

private final class ScriptedPlaces: MediaPlaces, @unchecked Sendable {
  let label: String
  private let events: Events
  private let answerHash: String?
  private let lock = NSLock()
  private var holds: [Gate] = []
  private let counters = ImageCounters()

  init(label: String, events: Events, answerHash: String? = nil) {
    self.label = label
    self.events = events
    self.answerHash = answerHash
  }

  func hold(_ gate: Gate) { lock.withLock { holds.append(gate) } }

  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder {
    let answer: Placeholder
    switch source.kind {
    case .image: answer = (answerHash ?? source.blurHash).map { .blurHash(hash: $0) } ?? .none
    case .video: answer = source.cover.map { .cover(url: $0) } ?? .firstFrame
    default: answer = .none
    }
    let gate = lock.withLock { holds.isEmpty ? nil : holds.removeFirst() }
    events.add(.ask(places: label, url: source.url, input: source.blurHash ?? source.cover, ready: ready, held: gate != nil))
    if let gate { await gate.wait() }
    events.add(.returned(places: label, url: source.url))
    return answer
  }

  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult {
    events.add(.fetch(url: url))
    let color: UIColor = url.contains("red") ? .red : url.contains("blue") ? .blue : .green
    let data = UIGraphicsImageRenderer(size: CGSize(width: 16, height: 16)).pngData { context in
      color.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 16, height: 16))
    }
    return ImageFetchResult(bytes: data, source: .network)
  }

  func imageStats() -> ImageCounters { counters }
  func cacheKey(url: String, headers: [String: String]) -> String { url }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { false }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute { throw CancellationError() }
  func stateStream(key: String) -> AsyncStream<MediaState> { AsyncStream { _ in } }
  func mountPlace(item: MediaItem) throws -> any PlaceHandle { throw MediaError.Closed }
  func mountVideoWindow() throws -> any WindowHandle { throw MediaError.Closed }
  func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle { throw MediaError.Closed }
  func seek(key: String, fraction: Double) throws {}
  func setMutedAll(muted: Bool) throws {}
  func setTonePreparation(preparation: TonePreparation) throws {}
}

private func tint(_ image: CGImage?) -> Tint {
  guard let image, let pixel = image.cropping(to: CGRect(x: image.width / 2, y: image.height / 2, width: 1, height: 1)) else { return .clear }
  var rgba = [UInt8](repeating: 0, count: 4)
  let context = CGContext(data: &rgba, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
  context?.draw(pixel, in: CGRect(x: 0, y: 0, width: 1, height: 1))
  let (r, g, b, a) = (Int(rgba[0]), Int(rgba[1]), Int(rgba[2]), Int(rgba[3]))
  if a < 30 { return .clear }
  if r > 180, g < 90, b < 90 { return .red }
  if b > 180, r < 90, g < 90 { return .blue }
  if g > 180, r < 90, b < 90 { return .green }
  return .other
}

private func image(_ hash: String?, url: String) -> MediaItem {
  MediaItem(kind: .image, url: url, headers: [:], cover: nil, blurHash: hash, strategy: .window)
}

private func video(_ cover: String?, url: String) -> MediaItem {
  MediaItem(kind: .video, url: url, headers: [:], cover: cover, blurHash: nil, strategy: .window)
}

@MainActor
private final class Mount {
  let events: Events
  let host = UIHostingController(rootView: AnyView(Color.clear))
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))

  init(events: Events) {
    self.events = events
    host.view.backgroundColor = .clear
    host.safeAreaRegions = []
    window.rootViewController = host
    window.isHidden = false
  }

  func show(_ item: MediaItem, ready: Bool = false, context: MediaContext) {
    let events = events
    host.rootView = AnyView(PlaceholderView(
      item: item,
      ready: ready,
      context: context,
      decode: { hash in
        events.add(.decode(hash: hash, onMain: Thread.isMainThread))
        return PlaceholderPixels.blurHash(hash)?.cgImage
      },
      published: { content in
        switch content {
        case .blur(let image): events.add(.published(.blur(tint(image))))
        case .cover(let image): events.add(.published(.cover(tint(image.cgImage))))
        case .none: events.add(.published(.none))
        }
      }
    ))
    flush()
  }

  func unmount() {
    host.rootView = AnyView(Color.clear)
    flush()
  }

  func flush() {
    host.view.frame = window.bounds
    host.view.layoutIfNeeded()
    CATransaction.flush()
  }

  func close() {
    window.isHidden = true
    window.rootViewController = nil
  }

  func shown() -> Tint {
    flush()
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let rendered = UIGraphicsImageRenderer(size: window.bounds.size, format: format).image { host.view.layer.render(in: $0.cgContext) }
    return tint(rendered.cgImage)
  }
}

@MainActor
private func environment(_ label: String, _ events: Events, answerHash: String? = nil) -> (ScriptedPlaces, MediaContext) {
  let places = ScriptedPlaces(label: label, events: events, answerHash: answerHash)
  ImageMemory.install(ImageCache(), on: places)
  return (places, MediaContext(places: places))
}

struct PlaceholderReplacement: CustomTestStringConvertible, @unchecked Sendable {
  let name: String
  let first: MediaItem
  let second: MediaItem
  let secondContext: Bool
  let oldInput: String
  var testDescription: String { name }
  var isVideo: Bool { first.kind == .video }
}

private let replacements: [PlaceholderReplacement] = [
  PlaceholderReplacement(name: "url", first: image(red, url: "https://o.test/a.png"), second: image(blue, url: "https://o.test/b.png"), secondContext: false, oldInput: red),
  PlaceholderReplacement(name: "blurHash same url", first: image(red, url: "https://o.test/same.png"), second: image(blue, url: "https://o.test/same.png"), secondContext: false, oldInput: red),
  PlaceholderReplacement(name: "cover same url", first: video("https://o.test/red-cover.png", url: "https://o.test/v.mp4"), second: video("https://o.test/blue-cover.png", url: "https://o.test/v.mp4"), secondContext: false, oldInput: "https://o.test/red-cover.png"),
  PlaceholderReplacement(name: "context same item", first: image(red, url: "https://o.test/ctx.png"), second: image(red, url: "https://o.test/ctx.png"), secondContext: true, oldInput: red)
]

struct PlaceholderViewTests {
  @MainActor
  @Test
  func inv_placeholder_blurHashDecodesOffTheMainThreadAndIsShown() async {
    let events = Events()
    let (_, context) = environment("A", events)
    let mount = Mount(events: events)
    defer { mount.close() }

    mount.show(image(red, url: "https://o.test/one.png"), context: context)
    guard await events.wait({ $0 == .published(.blur(.red)) }) else { Issue.record("blur never published"); return }

    #expect(events.all.contains(.decode(hash: red, onMain: false)))
    #expect(!events.all.contains(.decode(hash: red, onMain: true)))
    #expect(mount.shown() == .red)
  }

  @MainActor
  @Test(arguments: replacements)
  func inv_placeholder_aReplacedSourceNeverShowsOrDecodesTheOldAnswer(_ replacement: PlaceholderReplacement) async {
    let events = Events()
    let (first, firstContext) = environment("A", events)
    let mount = Mount(events: events)
    defer { mount.close() }
    let held = Gate()
    let newHeld = Gate()
    first.hold(held)
    let (second, secondContext) = environment("B", events, answerHash: replacement.secondContext ? blue : nil)
    (replacement.secondContext ? second : first).hold(newHeld)

    mount.show(replacement.first, context: firstContext)
    guard await events.wait({ if case .ask(let places, _, _, _, true) = $0 { return places == "A" }; return false }) else { Issue.record("old ask never arrived"); return }
    mount.show(replacement.second, context: replacement.secondContext ? secondContext : firstContext)
    let secondLabel = replacement.secondContext ? "B" : "A"
    guard await events.wait(atLeast: replacement.secondContext ? 1 : 2, { if case .ask(let places, _, _, _, true) = $0 { return places == secondLabel }; return false }) else { Issue.record("new ask never arrived"); return }
    #expect(mount.shown() == .clear)

    newHeld.open()
    let oldShown: Shown = replacement.isVideo ? .cover(.red) : .blur(.red)
    let newShown: Shown = replacement.isVideo ? .cover(.blue) : .blur(.blue)
    guard await events.wait({ $0 == .published(newShown) }) else { Issue.record("new answer never published"); return }
    #expect(mount.shown() == .blue)

    held.open()
    guard await events.wait({ $0 == .returned(places: "A", url: replacement.first.url) }) else { Issue.record("old answer never returned"); return }
    let control = environment("C", events)
    mount.unmount()
    mount.show(image(green, url: "https://o.test/control.png"), context: control.1)
    guard await events.wait({ $0 == .published(.blur(.green)) }) else { Issue.record("control never published"); return }

    let oldWork = events.all.filter { event in
      switch event {
      case .decode(let hash, _): return hash == replacement.oldInput
      case .fetch(let url): return url == replacement.oldInput
      default: return false
      }
    }
    #expect(oldWork == [])
    let publishedTints = events.all.compactMap { event -> Shown? in if case .published(let shown) = event { return shown }; return nil }
    #expect(!publishedTints.contains(oldShown))
    #expect(mount.shown() == .green)
  }

  @MainActor
  @Test(arguments: replacements)
  func inv_placeholder_aReplacedSourceDropsItsPublishedPictureAtOnce(_ replacement: PlaceholderReplacement) async {
    let events = Events()
    let (first, firstContext) = environment("A", events)
    let (second, secondContext) = environment("B", events, answerHash: replacement.secondContext ? blue : nil)
    let mount = Mount(events: events)
    defer { mount.close() }
    let oldShown: Shown = replacement.isVideo ? .cover(.red) : .blur(.red)
    let newShown: Shown = replacement.isVideo ? .cover(.blue) : .blur(.blue)

    mount.show(replacement.first, context: firstContext)
    guard await events.wait({ $0 == .published(oldShown) }) else { Issue.record("old picture never published"); return }
    #expect(mount.shown() == .red)

    let newHeld = Gate()
    (replacement.secondContext ? second : first).hold(newHeld)
    mount.show(replacement.second, context: replacement.secondContext ? secondContext : firstContext)
    guard await events.wait({ if case .ask(_, _, _, _, true) = $0 { return true }; return false }) else { Issue.record("new ask never arrived"); return }
    #expect(mount.shown() == .clear)

    newHeld.open()
    guard await events.wait({ $0 == .published(newShown) }) else { Issue.record("new answer never published"); return }
    #expect(mount.shown() == .blue)
  }

  @Test
  func inv_placeholder_aTaskCancelledBeforeTheDecodeStartsNeverDecodes() async {
    let events = Events()
    let gate = Gate()
    let task = Task { () -> CGImage? in
      await gate.wait()
      return await PlaceholderView.decodeOffMain(red) { hash in
        events.add(.decode(hash: hash, onMain: Thread.isMainThread))
        return PlaceholderPixels.blurHash(hash)?.cgImage
      }
    }

    task.cancel()
    gate.open()
    let image = await task.value

    #expect(image == nil)
    #expect(events.all == [])
  }

  @MainActor
  @Test
  func inv_placeholder_anUnmountedViewDoesNotDecodeALateAnswer() async {
    let events = Events()
    let (places, context) = environment("A", events)
    let (_, controlContext) = environment("C", events)
    let mount = Mount(events: events)
    defer { mount.close() }
    let held = Gate()
    places.hold(held)

    mount.show(image(red, url: "https://o.test/gone.png"), context: context)
    guard await events.wait({ if case .ask(_, _, _, _, true) = $0 { return true }; return false }) else { Issue.record("ask never arrived"); return }
    mount.unmount()
    held.open()
    guard await events.wait({ $0 == .returned(places: "A", url: "https://o.test/gone.png") }) else { Issue.record("answer never returned"); return }
    mount.show(image(green, url: "https://o.test/control.png"), context: controlContext)
    guard await events.wait({ $0 == .published(.blur(.green)) }) else { Issue.record("control never published"); return }

    #expect(events.count { if case .decode(let hash, _) = $0 { return hash == red }; return false } == 0)
    #expect(events.count { if case .published = $0 { return true }; return false } == 1)
  }

  @MainActor
  @Test
  func inv_placeholder_theSameSourceKeepsItsPictureUntilTheNextStepIsReady() async {
    let events = Events()
    let (places, context) = environment("A", events)
    let mount = Mount(events: events)
    defer { mount.close() }
    let url = "https://o.test/steps.png"

    mount.show(image(red, url: url), context: context)
    guard await events.wait({ $0 == .published(.blur(.red)) }) else { Issue.record("first step never published"); return }
    let held = Gate()
    places.hold(held)
    mount.show(image(red, url: url), ready: true, context: context)
    guard await events.wait({ if case .ask(_, _, _, true, true) = $0 { return true }; return false }) else { Issue.record("second step never asked"); return }

    #expect(mount.shown() == .red)
    held.open()
    guard await events.wait(atLeast: 2, { $0 == .published(.blur(.red)) }) else { Issue.record("second step never published"); return }
    #expect(mount.shown() == .red)
  }
}
#endif

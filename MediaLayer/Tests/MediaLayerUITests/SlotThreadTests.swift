import AVFoundation
import Foundation
import MediaLayer
import ObjectiveC
import Testing
@testable import MediaLayerUI

private final class ThreadCalls: @unchecked Sendable {
  private let lock = NSLock()
  private var values: [Bool] = []

  var all: [Bool] { lock.withLock { values } }
  func record() { lock.withLock { values.append(Thread.isMainThread) } }
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct SlotThreadTests {
  @Test
  func inv_api_33_sample_does_not_wait_for_create() async throws {
    let calls = ThreadCalls()
    let selector = NSSelectorFromString("setVideoGravity:")
    let method = try #require(class_getInstanceMethod(AVPlayerLayer.self, selector))
    let original = method_getImplementation(method)
    typealias Setter = @convention(c) (AnyObject, Selector, NSString) -> Void
    let intercept: @convention(block) (AnyObject, NSString) -> Void = { object, gravity in
      calls.record()
      unsafeBitCast(original, to: Setter.self)(object, selector, gravity)
    }
    let replacement = imp_implementationWithBlock(intercept)
    method_setImplementation(method, replacement)
    defer {
      method_setImplementation(method, original)
      imp_removeBlock(replacement)
    }
    let slots = LoadedSlots()
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: slots)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let media = try MediaLayer.open(config: OpenConfig(
      dir: directory.path,
      availableBytes: UInt64.max, fetcher: URLSessionFetcher(),
      settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
      slots: factory,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    factory.events.bind(media.playerEvents())
    let url = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    media.visible(source: MediaItem(kind: .video, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window), visible: true)
    await eventually("the core creates and loads the native slot") { slots.slot(for: url) != nil }
    #expect(!calls.all.isEmpty)
    #expect(calls.all.allSatisfy { $0 }, "native initialization ran outside main")
    let slot = try #require(slots.slot(for: url))
    let sampleCalls = ThreadCalls()
    let itemSelector = NSSelectorFromString("currentItem")
    let itemMethod = try #require(class_getInstanceMethod(AVPlayer.self, itemSelector))
    let originalItem = method_getImplementation(itemMethod)
    typealias Getter = @convention(c) (AnyObject, Selector) -> AnyObject?
    let player = slot.player
    let read: @convention(block) (AnyObject) -> AnyObject? = { object in
      if object === player { sampleCalls.record() }
      return unsafeBitCast(originalItem, to: Getter.self)(object, itemSelector)
    }
    let replacementItem = imp_implementationWithBlock(read)
    method_setImplementation(itemMethod, replacementItem)
    let sample = await Task.detached { slot.sample() }.value
    method_setImplementation(itemMethod, originalItem)
    imp_removeBlock(replacementItem)
    #expect(sample.positionMs == 0)
    #expect(sampleCalls.all.allSatisfy { $0 }, "sample read the native player outside main")
    media.releaseAll()
    factory.events.bind(nil)
  }
}

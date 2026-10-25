import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private func gif(frames: Int, side: Int) -> Data {
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  for index in 0 ..< frames {
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: CGFloat(index % 5) / 5, green: 0, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: side, height: side))
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

private final class Live: @unchecked Sendable {
  private let lock = NSLock()
  private var count = 0
  let freed = AsyncStream<Void>.makeStream()
  var current: Int { lock.withLock { count } }
  func add() { lock.withLock { count += 1 } }
  func drop() {
    lock.withLock { count -= 1 }
    freed.continuation.yield(())
  }
}

private func tracked(_ bytes: Data, _ live: Live) -> Data {
  let pointer = UnsafeMutableRawPointer.allocate(byteCount: bytes.count, alignment: 1)
  bytes.copyBytes(to: pointer.assumingMemoryBound(to: UInt8.self), count: bytes.count)
  live.add()
  return Data(bytesNoCopy: pointer, count: bytes.count, deallocator: .custom { raw, _ in
    raw.deallocate()
    live.drop()
  })
}

private final class Barrier: @unchecked Sendable {
  private let lock = NSLock()
  private var held: CheckedContinuation<Void, Never>?
  private var opened = false

  func pass(releaseOnCancel: Bool = false) async {
    await withTaskCancellationHandler {
      await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        let open = lock.withLock { () -> Bool in
          if !opened { held = continuation }
          return opened
        }
        if open { continuation.resume() }
      }
    } onCancel: { if releaseOnCancel { open() } }
  }

  func open() {
    let waiting = lock.withLock { () -> CheckedContinuation<Void, Never>? in
      opened = true
      defer { held = nil }
      return held
    }
    waiting?.resume()
  }
}

private final class Probe: @unchecked Sendable {
  private let lock = NSLock()
  private var inside = 0
  private var returned = 0
  private var cancelledAtExit: Bool?
  private var exitFrames: Bool?
  var work: Int { lock.withLock { inside } }
  var callersReturned: Int { lock.withLock { returned } }
  var exitedCancelled: Bool? { lock.withLock { cancelledAtExit } }
  var exitedWithFrames: Bool? { lock.withLock { exitFrames } }
  func enter() { lock.withLock { inside += 1 } }
  func leave(cancelled: Bool, frames: Bool) { lock.withLock { inside -= 1; cancelledAtExit = cancelled; exitFrames = frames } }
  func callerReturned() { lock.withLock { returned += 1 } }
}

struct DecodeCancellationTests {
  @Test
  func inv_native_23_cancelled_callers_return_before_the_decode_ends() async throws {
    let live = Live()
    let barrier = Barrier()
    let probe = Probe()
    let started = AsyncStream<Void>.makeStream()
    let encoded = gif(frames: 30, side: 16)
    let cache = FrameCache { data, maxPixel in
      probe.enter()
      let payload = data
      started.continuation.yield(())
      await barrier.pass()
      let frames = await AnimatedFrames.load(payload, maxPixel: maxPixel)
      probe.leave(cancelled: Task.isCancelled, frames: frames != nil)
      return frames
    }
    let payload = tracked(encoded, live)
    let callers = (0 ..< 3).map { _ in
      Task {
        let frames = await cache.load(identity: "a", data: payload, maxPixel: 16)
        probe.callerReturned()
        return frames
      }
    }
    var begins = started.stream.makeAsyncIterator()
    _ = await begins.next()
    while cache.waiters < 3 { await Task.yield() }

    callers.forEach { $0.cancel() }
    for _ in 0 ..< 200 where probe.callersReturned < 3 { await Task.yield() }
    let returnedWhileHeld = probe.callersReturned
    let workWhileHeld = probe.work
    barrier.open()
    for caller in callers { #expect(await caller.value == nil) }

    #expect(returnedWhileHeld == 3)
    #expect(workWhileHeld == 1)
    #expect(cache.waiters == 0)
  }

  @Test
  func inv_native_24_a_decode_without_waiters_exits_by_cancel_and_frees_its_payload() async throws {
    let live = Live()
    let barrier = Barrier()
    let probe = Probe()
    let started = AsyncStream<Void>.makeStream()
    let exited = AsyncStream<Void>.makeStream()
    let cache = FrameCache { data, maxPixel in
      probe.enter()
      let payload = data
      started.continuation.yield(())
      await barrier.pass(releaseOnCancel: true)
      let frames = await AnimatedFrames.load(payload, maxPixel: maxPixel)
      probe.leave(cancelled: Task.isCancelled, frames: frames != nil)
      exited.continuation.yield(())
      return frames
    }
    let uncancelled = FrameCache()
    let encoded = gif(frames: 30, side: 16)
    #expect(try #require(await uncancelled.load(identity: "full", data: encoded, maxPixel: 16)).frameCount == 30)
    do {
      let caller = Task { await cache.load(identity: "a", data: tracked(encoded, live), maxPixel: 16) }
      var begins = started.stream.makeAsyncIterator()
      _ = await begins.next()
      while cache.waiters < 1 { await Task.yield() }
      #expect(live.current == 1)

      caller.cancel()
      _ = await caller.value
      for _ in 0 ..< 200 where probe.work > 0 { await Task.yield() }
      barrier.open()
    }
    var exits = exited.stream.makeAsyncIterator()
    _ = await exits.next()
    var releases = live.freed.stream.makeAsyncIterator()
    _ = await releases.next()

    #expect(probe.exitedCancelled == true)
    #expect(probe.exitedWithFrames == false)
    #expect(probe.work == 0)
    #expect(live.current == 0)
    #expect(cache.count == 0)
    #expect(cache.bytes == 0)
  }
}

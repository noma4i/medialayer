import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI
@testable import MediaLayer

extension FrameCache {
  func load(identity: String, data: Data, maxPixel: Int) async -> AnimatedFrames? {
    await load(identity: identity, data: data, maxPixel: maxPixel, admit: { _ in {} })
  }
}

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

private func png(width: Int, height: Int) throws -> URL {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ))
  let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
  let destination = try #require(CGImageDestinationCreateWithURL(file as CFURL, UTType.png.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), nil)
  #expect(CGImageDestinationFinalize(destination))
  return file
}

private final class Tally: @unchecked Sendable {
  private let lock = NSLock()
  private var items: [UInt64] = []
  private var releases = 0
  private var running = 0
  private var peakRunning = 0
  private var held = 0
  private var waiting: [CheckedContinuation<Void, Never>] = []
  let capacity: Int

  init(capacity: Int = .max) { self.capacity = capacity }

  var asked: [UInt64] { lock.withLock { items } }
  var released: Int { lock.withLock { releases } }
  var peak: Int { lock.withLock { peakRunning } }

  func take(_ bytes: UInt64) async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
      let granted = lock.withLock { () -> Bool in
        items.append(bytes)
        guard held < capacity else {
          waiting.append(continuation)
          return false
        }
        held += 1
        return true
      }
      if granted { continuation.resume() }
    }
  }

  func give() {
    let next = lock.withLock { () -> CheckedContinuation<Void, Never>? in
      releases += 1
      if waiting.isEmpty {
        held -= 1
        return nil
      }
      return waiting.removeFirst()
    }
    next?.resume()
  }

  func begin() { lock.withLock { running += 1; peakRunning = max(peakRunning, running) } }
  func end() { lock.withLock { running -= 1 } }
}

private final class ThumbnailCalls: @unchecked Sendable {
  private let lock = NSLock()
  private var calls = 0
  var count: Int { lock.withLock { calls } }
  func add() { lock.withLock { calls += 1 } }
}

private struct Poster: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

struct DecodeAdmissionTests {
  @Test
  func inv_memory_16_the_cache_cost_counts_the_encoded_source_next_to_the_decoded_frames() async throws {
    let data = gif(frames: 3, side: 16)
    let frames = try #require(AnimatedFrames.decode(data, maxPixel: 16))
    let window = frames.firstImage.bytesPerRow * frames.firstImage.height * FrameStream.window
    #expect(frames.retainedBytes == window + data.count)
    let cache = FrameCache()
    _ = try #require(await cache.load(identity: "a", data: data, maxPixel: 16))
    #expect(cache.bytes == window + data.count)
  }

  @Test
  func inv_memory_17_gif_decodes_run_only_inside_the_ledger_admission() async throws {
    let data = gif(frames: 3, side: 16)
    let tally = Tally(capacity: 2)
    let cache = FrameCache(decode: { data, size in
      tally.begin()
      defer { tally.end() }
      try? await Task.sleep(for: .milliseconds(30))
      return AnimatedFrames.decode(data, maxPixel: size)
    })
    let admit: FrameCache.Admit = { bytes in
      await tally.take(bytes)
      return { tally.give() }
    }
    let loaded = await withTaskGroup(of: AnimatedFrames?.self) { group in
      for index in 0 ..< 6 { group.addTask { await cache.load(identity: "k\(index)", data: data, maxPixel: 16, admit: admit) } }
      return await group.reduce(into: 0) { $0 += $1 == nil ? 0 : 1 }
    }
    #expect(loaded == 6)
    #expect(tally.peak <= 2)
    #expect(tally.released == 6)
    #expect(tally.asked == Array(repeating: AnimatedFrames.decodeBytes(sourceBytes: data.count, maxPixel: 16), count: 6))
  }

  @Test
  func inv_memory_17_a_refused_admission_ends_the_load_without_a_decode() async throws {
    let data = gif(frames: 3, side: 16)
    let tally = Tally()
    let cache = FrameCache(decode: { data, size in
      tally.begin()
      tally.end()
      return AnimatedFrames.decode(data, maxPixel: size)
    })
    let frames = await cache.load(identity: "a", data: data, maxPixel: 16, admit: { _ in nil })
    #expect(frames == nil)
    #expect(tally.peak == 0)
    #expect(cache.count == 0)
  }

  private func viewDecoder(pixels: CGSize?, admit: @escaping FrameCache.Admit) throws -> (AnimatedKeepingDecoder, Data) {
    let file = try png(width: 40, height: 20)
    defer { try? FileManager.default.removeItem(at: file) }
    let data = try Data(contentsOf: file)
    var request = ImageRequest(url: URL(string: "https://o.test/a.png"))
    if let pixels { request.userInfo["decodePixels"] = pixels }
    let decoder = try #require(AnimatedKeepingDecoder(context: ImageDecodingContext(request: request, data: data), admit: admit))
    return (decoder, data)
  }

  @Test
  func inv_memory_13_a_view_decode_asks_the_ledger_for_the_requested_pixels_and_gives_them_back() async throws {
    let tally = Tally()
    let (decoder, data) = try viewDecoder(pixels: CGSize(width: 20, height: 10)) { bytes in
      await tally.take(bytes)
      return { tally.give() }
    }

    _ = try await decoder.decode(data)

    #expect(tally.asked == [20 * 10 * 4])
    #expect(tally.released == 1)
  }

  @Test
  func inv_memory_13_a_view_decode_without_requested_pixels_asks_for_the_size_in_the_header() async throws {
    let tally = Tally()
    let (decoder, data) = try viewDecoder(pixels: nil) { bytes in
      await tally.take(bytes)
      return { tally.give() }
    }

    _ = try await decoder.decode(data)

    #expect(tally.asked == [40 * 20 * 4])
  }

  @Test
  func inv_memory_13_a_refused_admission_stops_the_view_decode_with_an_error() async throws {
    let (decoder, data) = try viewDecoder(pixels: CGSize(width: 20, height: 10)) { _ in nil }

    await #expect(throws: MediaError.self) { _ = try await decoder.decode(data) }
  }

  @Test
  func inv_memory_13_a_view_decode_above_the_pixel_limit_is_refused_before_any_admission() async throws {
    let tally = Tally()
    let (decoder, data) = try viewDecoder(pixels: CGSize(width: 16_385, height: 1)) { bytes in
      await tally.take(bytes)
      return { tally.give() }
    }

    await #expect(throws: MediaError.self) { _ = try await decoder.decode(data) }
    #expect(tally.asked.isEmpty)
  }

  @Test
  func inv_image_65_a_webp_keeps_its_source_bytes_only_when_it_has_more_than_one_frame() {
    #expect(AnimatedFrames.sourceFrameCount(gif(frames: 1, side: 8)) == 1)
    #expect(AnimatedFrames.sourceFrameCount(gif(frames: 3, side: 8)) == 3)
    #expect(!AnimatedKeepingDecoder.keepsSource(frames: 1))
    #expect(AnimatedKeepingDecoder.keepsSource(frames: 3))
  }

  @Test
  func inv_tone_21_a_cancelled_tone_never_reaches_the_thumbnail_decode() async throws {
    let url = "https://o.test/p.png"
    let file = try png(width: 8, height: 8)
    let places = RecordingPlaces()
    places.needsTone = true
    places.answer(url, with: try Data(contentsOf: file))
    let calls = ThumbnailCalls()
    let tone = ToneFrames(places: places, thumbnail: { data in calls.add(); return ToneFrames.thumbnail(data) })
    let item = Poster(url: url).item

    #expect(await tone.prepare(source: item) != nil)
    #expect(calls.count == 1)

    let latch = Latch()
    places.fetchLatch = latch
    let work = Task { await tone.prepare(source: item) }
    for _ in 0 ..< 10_000 where places.fetchedURLs.count < 2 { await Task.yield() }
    work.cancel()
    latch.open()
    #expect(await work.value == nil)
    #expect(calls.count == 1)
  }
}

extension GlobalMedia {
  struct FrameAdmissionFailureTests {
    @Test
    func inv_memory_17_a_refusal_of_the_ledger_reaches_media_failure_and_a_cancel_does_not() async {
      Media.clearFailure()
      defer { Media.clearFailure() }
      let cancelled = await FrameCache.admit(10) { _ in throw CancellationError() }
      #expect(cancelled == nil)
      #expect(Media.failure == nil)

      let refused = await FrameCache.admit(10) { _ in throw MediaError.Overloaded(detail: "no room") }
      #expect(refused == nil)
      #expect(Media.failure != nil)
      Media.clearFailure()

      let granted = await FrameCache.admit(10) { _ in {} }
      #expect(granted != nil)
      #expect(Media.failure == nil)
    }
  }
}

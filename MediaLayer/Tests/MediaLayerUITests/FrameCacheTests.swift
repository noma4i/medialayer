import CoreGraphics
import Foundation
import ImageIO
import Nuke
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private func gif(frames: Int, side: Int, seed: Int) -> Data {
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  for index in 0 ..< frames {
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: CGFloat((index + seed) % 5) / 5, green: 0, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: side, height: side))
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

struct FrameCacheTests {
  @Test
  func theSameGifAppearingTwiceIsDecodedOnceAndTheSecondAppearanceGetsTheSameFrames() async throws {
    let cache = FrameCache()
    let data = gif(frames: 30, side: 32, seed: 0)

    let first = try #require(await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 32))
    let second = try #require(await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 32))

    #expect(cache.decodes == 1)
    #expect(second.token == first.token)
    #expect(second.frameCount == 30)
    _ = try #require(await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 16))
    #expect(cache.decodes == 2)
  }

  @Test
  func twentyGifsNeverHoldMoreThanTheByteLimitAndTheOldestGoesFirst() async throws {
    let limit = 1 << 20
    let cache = FrameCache(byteLimit: limit)
    for index in 0 ..< 20 {
      _ = await cache.load(url: "https://o.test/\(index).gif", data: gif(frames: 40, side: 64, seed: index), maxPixel: 64)
      #expect(cache.bytes <= limit)
    }
    let decoded = cache.decodes

    _ = await cache.load(url: "https://o.test/19.gif", data: gif(frames: 40, side: 64, seed: 19), maxPixel: 64)
    #expect(cache.decodes == decoded)
    _ = await cache.load(url: "https://o.test/0.gif", data: gif(frames: 40, side: 64, seed: 0), maxPixel: 64)
    #expect(cache.decodes == decoded + 1)
  }

  @Test
  func aMemoryPressureSignalEmptiesTheCache() async throws {
    let cache = FrameCache()
    let data = gif(frames: 10, side: 32, seed: 0)
    _ = await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 32)
    #expect(cache.count == 1)

    ImageMemoryReleaser(cache: ImageCache(), frames: cache).dropMemoryCaches()

    #expect(cache.count == 0)
    #expect(cache.bytes == 0)
  }

  @Test
  func twoWaitersOfOneGifShareOneDecode() async throws {
    let cache = FrameCache()
    let data = gif(frames: 120, side: 64, seed: 0)

    async let first = cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64)
    async let second = cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64)
    let frames = try await (#require(first), #require(second))

    #expect(cache.decodes == 1)
    #expect(frames.0.token == frames.1.token)
  }

  @Test
  func aWaiterThatLeavesDoesNotCancelTheDecodeOfTheOtherWaiter() async throws {
    let cache = FrameCache()
    let data = gif(frames: 1200, side: 64, seed: 0)
    let leaving = Task { await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64) }
    let staying = Task { await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64) }
    await eventually("the decode started") { cache.decodes > 0 }

    leaving.cancel()

    #expect(await staying.value?.frameCount == 1200)
    #expect(cache.decodes == 1)
    #expect(cache.count == 1)
  }

  @Test
  func whenTheLastWaiterLeavesTheDecodeStopsAndStoresNothing() async throws {
    let cache = FrameCache()
    let data = gif(frames: 1200, side: 64, seed: 0)
    let only = Task { await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64) }
    await eventually("the decode started") { cache.decodes > 0 }

    only.cancel()

    #expect(await only.value == nil)
    #expect(cache.count == 0)
    #expect(await cache.load(url: "https://o.test/a.gif", data: data, maxPixel: 64) != nil)
    #expect(cache.decodes == 2)
  }
}

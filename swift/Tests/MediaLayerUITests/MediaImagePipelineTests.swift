import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private struct TestImage: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] = [:]
  var size: MediaSize?
}

private func pixelWidth(_ image: PlatformImage) -> Int? {
  #if canImport(UIKit)
  image.cgImage?.width
  #else
  image.cgImage(forProposedRect: nil, context: nil, hints: nil)?.width
  #endif
}

extension GlobalMedia {
  @Suite(.timeLimit(.minutes(1)))
  struct MediaImagePipelineTests {
    @Test(arguments: [(MediaSize?.some(MediaSize(width: 8, height: 8)), 1), (MediaSize?.none, 0)])
    func requestResizesToTheDeclaredSizeAndForwardsHeaders(size: MediaSize?, processors: Int) throws {
      let request = try #require(MediaImagePipeline.request(for: TestImage(url: "https://o.test/a.png", headers: ["X-Token": "t"], size: size)))

      #expect(request.processors.count == processors)
      #expect(request.urlRequest?.value(forHTTPHeaderField: "X-Token") == "t")
    }

    @Test
    func imageLoadsThroughTheCoreAndASecondPipelineReadsTheCoreDiskCacheWithTheOriginDown() async throws {
      let server = try LocalHTTPServer()
      try await server.start()
      server.serve(pngData(side: 16), at: "/image.png")
      let media = try MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      let source = TestImage(url: server.url("/image.png"), headers: ["Authorization": "Bearer t"], size: MediaSize(width: 4, height: 4))
      let request = try #require(MediaImagePipeline.request(for: source))

      let first = try await MediaImagePipeline.make(places: media).image(for: request)
      server.stop()
      let second = try await MediaImagePipeline.make(places: media).image(for: request)

      #expect(pixelWidth(first) == 4)
      #expect(pixelWidth(second) == 4)
      #expect(server.headers(of: "/image.png")["authorization"] == "Bearer t")
      #expect(MediaImagePipeline.make(places: media).configuration.dataCache == nil)
    }
  }
}

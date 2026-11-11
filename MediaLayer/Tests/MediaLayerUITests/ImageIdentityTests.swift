import Foundation
import MediaLayer
import Nuke
import Testing
@testable import MediaLayerUI

private struct AccountImage: MediaSource {
  let url = "https://o.test/account.png"
  var kind: Kind { .image }
  let headers: [String: String]
}

private final class IdentityBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private var data: Data?
  init(_ data: Data) { self.data = data }
  func nextChunk() async throws -> Data? { lock.withLock { defer { data = nil }; return data } }
  func release() {}
}

private final class IdentityOrigin: Fetcher, @unchecked Sendable {
  private let lock = NSLock()
  private var accounts: [String] = []
  var seen: [String] { lock.withLock { accounts } }
  func fetch(request: FetchRequest) async throws -> FetchResponse {
    let account = request.headers.first { $0.key.lowercased() == "authorization" }?.value ?? ""
    lock.withLock { accounts.append(account) }
    return FetchResponse(status: 200, headers: ["Content-Type": "image/png"], body: IdentityBody(pngData(side: account == "A" ? 8 : 16)))
  }
}

private final class IdentitySettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

struct ImageIdentityTests {
  @Test
  func selectedHeadersSeparateDecodedImagesAndUnselectedHeadersReuseThem() async throws {
    let origin = IdentityOrigin()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: origin, settings: IdentitySettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: ["Authorization"]
    ))
    ImageMemory.install(ImageCache(), on: media)
    let pipeline = MediaImagePipeline.make(places: media)
    for (account, ignored, expectedWidth) in [("A", "1", 8), ("B", "1", 16), ("A", "2", 8)] {
      let source = AccountImage(headers: ["Authorization": account, "X-Ignored": ignored])
      let request = try #require(MediaImagePipeline.request(places: media, for: source))
      let image = try await pipeline.image(for: request)
      #expect(Int(image.size.width) == expectedWidth)
    }
    #expect(origin.seen == ["A", "B"])
  }
}

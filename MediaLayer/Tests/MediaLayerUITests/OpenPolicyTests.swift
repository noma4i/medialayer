import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class PolicyBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private var bytes: Data?

  init(count: Int) { bytes = Data(repeating: 7, count: count) }

  func nextChunk() async throws -> Data? {
    lock.withLock {
      defer { bytes = nil }
      return bytes
    }
  }

  func release() {}
}

private final class PolicyOrigin: Fetcher, @unchecked Sendable {
  private let lock = NSLock()
  private var requests: [FetchRequest] = []
  var seen: [FetchRequest] { lock.withLock { requests } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { requests.append(request) }
    let count = request.url.contains("large") ? 7 : request.url.contains("new") ? 4 : 2
    return FetchResponse(status: 200, headers: ["Content-Length": "\(count)"], body: PolicyBody(count: count))
  }
}

extension GlobalMedia {
  struct OpenPolicyTests {
    private func open(_ spec: String, origin: PolicyOrigin) async throws -> MediaLayer {
      try await MediaLayer.open(
        spec: spec, fetcher: origin,
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
    }

    @Test
    func inv_spec_07_open_applies_the_first_video_forward_headers_to_origin() async throws {
      let cases: [(String, [String: String])] = [
        (#"{"videos":[{"name":"first","forward_headers":["X-Token"]},{"name":"second","forward_headers":["Authorization"]}]}"#, ["X-Token": "selected"]),
        (#"{"videos":[{"name":"first","forward_headers":[]}]}"#, [:]),
        ("{}", ["Authorization": "excluded"])
      ]
      for (spec, expected) in cases {
        let origin = PolicyOrigin()
        let media = try await open(spec, origin: origin)
        let bytes = try await media.fetchImage(url: "https://o.test/small.jpg", headers: ["X-Token": "selected", "Authorization": "excluded"])

        #expect(bytes.bytes.count == 2)
        #expect(origin.seen.first?.headers == expected.merging(["Accept-Encoding": "identity"]) { _, value in value })
      }
    }

    @Test
    func inv_spec_07_open_applies_the_first_eviction_override_of_each_kind() async throws {
      for (section, suffix) in [("videos", "mp4"), ("images", "jpg"), ("audios", "mp3")] {
        let origin = PolicyOrigin()
        let media = try await open(
          #"{"cache":{"budget":10,"min":1,"evict":"lru"},"\#(section)":[{"name":"first","evict":"largest_first"},{"name":"second","evict":"fifo"}]}"#,
          origin: origin
        )
        for name in ["small", "large", "new", "small"] {
          _ = try await media.fetchImage(url: "https://o.test/\(name).\(suffix)", headers: [:])
        }
        #expect(origin.seen.filter { $0.url.contains("small") }.count == 1)
      }
    }
  }
}

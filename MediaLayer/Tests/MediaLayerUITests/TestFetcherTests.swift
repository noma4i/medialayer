import Foundation
import MediaLayer
import MediaLayerTesting
import Testing

struct TestFetcherTests {
  private let payload = Data((0..<10_000).map { UInt8($0 % 251) })
  private let url = "https://o.test/a.mp4"

  private func request(_ url: String, _ start: UInt64? = nil, _ end: UInt64? = nil) -> FetchRequest {
    FetchRequest(url: url, headers: [:], rangeStart: start, rangeEnd: end)
  }

  private func read(_ response: FetchResponse) async throws -> Data {
    var out = Data()
    while let chunk = try await response.body.nextChunk() { out.append(chunk) }
    return out
  }

  @Test
  func inv_native_32_testFetcherAnswersRangesLikeAnOrigin() async throws {
    let fetcher = TestFetcher()
    fetcher.serve(payload, at: url, contentType: "video/mp4")

    let whole = try await fetcher.fetch(request: request(url))
    #expect(whole.status == 200)
    #expect(whole.headers["Content-Type"] == "video/mp4")
    #expect(try await read(whole) == payload)

    let middle = try await fetcher.fetch(request: request(url, 10, 19))
    #expect(middle.status == 206)
    #expect(middle.headers["Content-Range"] == "bytes 10-19/10000")
    #expect(try await read(middle) == payload.subdata(in: 10..<20))

    let tail = try await fetcher.fetch(request: request(url, 9_990, nil))
    #expect(tail.headers["Content-Range"] == "bytes 9990-9999/10000")
    #expect(try await read(tail) == payload.subdata(in: 9_990..<10_000))

    let clamped = try await fetcher.fetch(request: request(url, 9_990, 50_000))
    #expect(clamped.headers["Content-Range"] == "bytes 9990-9999/10000")
    #expect(try await read(clamped).count == 10)

    let beyond = try await fetcher.fetch(request: request(url, 10_000, nil))
    #expect(beyond.status == 416)
    #expect(beyond.headers["Content-Range"] == "bytes */10000")
    #expect(try await read(beyond).isEmpty)
  }

  @Test
  func inv_native_33_testFetcherCountsRequestsAndAppliesFailureAndDelayPerUrl() async throws {
    let fetcher = TestFetcher()
    fetcher.serve(payload, at: url)
    fetcher.serve(payload, at: "https://o.test/slow")
    fetcher.delay(.seconds(60), for: "https://o.test/slow")
    fetcher.fail("https://o.test/bad", detail: "boom")

    _ = try await fetcher.fetch(request: request(url))
    _ = try await fetcher.fetch(request: request(url, 0, 1))
    await #expect(throws: MediaError.self) { try await fetcher.fetch(request: request("https://o.test/bad")) }
    await #expect(throws: MediaError.self) { try await fetcher.fetch(request: request("https://o.test/unknown")) }
    let slow = Task { try await fetcher.fetch(request: request("https://o.test/slow")) }
    try await Task.sleep(for: .milliseconds(50))
    slow.cancel()
    await #expect(throws: CancellationError.self) { try await slow.value }

    #expect(fetcher.requestCount(for: url) == 2)
    #expect(fetcher.requestCount(for: "https://o.test/bad") == 1)
    #expect(fetcher.requestCount(for: "https://o.test/slow") == 1)
    #expect(fetcher.requests.count == 5)
  }
}

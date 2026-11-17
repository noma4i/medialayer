import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct URLSessionFetcherTests {
  private func serve() async throws -> LocalHTTPServer {
    let server = try LocalHTTPServer()
    try await server.start()
    return server
  }

  private func request(_ url: String, headers: [String: String] = [:], from: UInt64? = nil, to: UInt64? = nil) -> FetchRequest {
    FetchRequest(url: url, headers: headers, rangeStart: from, rangeEnd: to)
  }

  private func drain(_ body: BodyReader) async throws -> Data {
    var all = Data()
    while let chunk = try await body.nextChunk() { all.append(chunk) }
    return all
  }

  @Test
  func bodyArrivesInChunksBeforeTheServerFinishes() async throws {
    let server = try await serve()
    defer { server.stop() }
    let fetcher = URLSessionFetcher()
    let response = try await fetcher.fetch(request: request(server.url("/gated")))

    let first = try await response.body.nextChunk()

    #expect(response.status == 200)
    let head = try #require(first)
    #expect(!head.isEmpty && head.count < LocalHTTPServer.payload.count)
    server.openGate()
    let rest = try await drain(response.body)
    #expect(head + rest == LocalHTTPServer.payload)
  }

  @Test(arguments: [
    (UInt64(10), UInt64?.some(19), 10...19),
    (UInt64(990), UInt64?.none, 990...999)
  ])
  func rangeBecomesARangeHeader(start: UInt64, end: UInt64?, expected: ClosedRange<Int>) async throws {
    let server = try await serve()
    defer { server.stop() }

    let fetcher = URLSessionFetcher()
    let response = try await fetcher.fetch(request: request(server.url("/bytes"), from: start, to: end))

    #expect(response.status == 206)
    #expect(try await drain(response.body) == LocalHTTPServer.payload[expected])
  }

  @Test
  func requestHeadersReachTheServerAndResponseHeadersComeBack() async throws {
    let server = try await serve()
    defer { server.stop() }

    let fetcher = URLSessionFetcher()
    let response = try await fetcher.fetch(request: request(server.url("/headers"), headers: ["X-Token": "abc"]))

    let echoed = String(decoding: try await drain(response.body), as: UTF8.self)
    #expect(echoed.contains("x-token: abc"))
    #expect(response.headers["Content-Length"] == "\(echoed.utf8.count)")
  }

  @Test
  func cancelledFetchClosesTheConnection() async throws {
    let server = try await serve()
    defer { server.stop() }
    let fetcher = URLSessionFetcher(timeout: 600)
    let url = server.url("/hang")
    let pending = Task { try await fetcher.fetch(request: request(url)) }
    var closed = server.closed.makeAsyncIterator()
    var arrived = server.requests.makeAsyncIterator()
    _ = await arrived.next()

    pending.cancel()

    await #expect(throws: MediaError.self) { try await pending.value }
    #expect(await closed.next() == "/hang")
  }

  @Test
  func droppingTheBodyMidStreamStopsTheDownload() async throws {
    let server = try await serve()
    defer { server.stop() }
    let fetcher = URLSessionFetcher()
    var closed = server.closed.makeAsyncIterator()

    try await readFirstChunkAndDrop(fetcher, request(server.url("/endless")))

    #expect(await closed.next() == "/endless")
  }

  private func readFirstChunkAndDrop(_ fetcher: URLSessionFetcher, _ fetch: FetchRequest) async throws {
    let response = try await fetcher.fetch(request: fetch)
    _ = try await response.body.nextChunk()
  }

  @Test
  func releaseStopsTheDownloadAndEndsTheBody() async throws {
    let server = try await serve()
    defer { server.stop() }
    let fetcher = URLSessionFetcher()
    let response = try await fetcher.fetch(request: request(server.url("/endless")))
    var closed = server.closed.makeAsyncIterator()
    _ = try await response.body.nextChunk()

    response.body.release()

    #expect(await closed.next() == "/endless")
    #expect(try await response.body.nextChunk() == nil)
  }

  @Test
  func silentServerFailsAfterTheTimeout() async throws {
    let server = try await serve()
    defer { server.stop() }

    await #expect(throws: MediaError.self) {
      try await URLSessionFetcher(timeout: 0.5).fetch(request: request(server.url("/hang")))
    }
  }

  @Test
  func bodyPausesTheDownloadAtTheLimitAndResumesAfterDraining() async throws {
    let events = BodyEvents()
    let body = StreamingBody(limit: 100_000, pause: events.paused, resume: events.resumed, cancel: {})

    body.receive(Data(count: 40_000))
    body.receive(Data(count: 40_000))
    body.receive(Data(count: 40_000))
    #expect(events.counts == (1, 0))
    #expect(try await body.nextChunk()?.count == 40_000)
    #expect(events.counts == (1, 0))
    #expect(try await body.nextChunk()?.count == 40_000)
    #expect(events.counts == (1, 1))
    #expect(try await body.nextChunk()?.count == 40_000)
    #expect(events.counts == (1, 1))
  }
}

private final class BodyEvents: @unchecked Sendable {
  private let lock = NSLock()
  private var pauses = 0
  private var resumes = 0

  var counts: (Int, Int) { lock.withLock { (pauses, resumes) } }
  var paused: @Sendable () -> Void { { self.lock.withLock { self.pauses += 1 } } }
  var resumed: @Sendable () -> Void { { self.lock.withLock { self.resumes += 1 } } }
}

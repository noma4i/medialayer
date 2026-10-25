import Foundation
import MediaLayer

public final class TestFetcher: Fetcher, @unchecked Sendable {
  private enum Content {
    case bytes(Data)
    case file(URL)
  }

  private struct Entry {
    var content: Content
    var contentType: String?
  }

  private let lock = NSLock()
  private let chunkSize: Int
  private var entries: [String: Entry] = [:]
  private var delays: [String: Duration] = [:]
  private var failures: [String: String] = [:]
  private var seen: [FetchRequest] = []

  public init(chunkSize: Int = 4096) {
    self.chunkSize = chunkSize
  }

  public var requests: [FetchRequest] { lock.withLock { seen } }

  public func requestCount(for url: String) -> Int { lock.withLock { seen.filter { $0.url == url }.count } }

  public func serve(_ data: Data, at url: String, contentType: String? = nil) {
    lock.withLock { entries[url] = Entry(content: .bytes(data), contentType: contentType) }
  }

  public func serveFile(_ file: URL, at url: String, contentType: String? = nil) {
    lock.withLock { entries[url] = Entry(content: .file(file), contentType: contentType) }
  }

  public func delay(_ interval: Duration, for url: String) {
    lock.withLock { delays[url] = interval }
  }

  public func fail(_ url: String, detail: String = "offline") {
    lock.withLock { failures[url] = detail }
  }

  public func fetch(request: FetchRequest) async throws -> FetchResponse {
    let (entry, delay, failure) = lock.withLock {
      seen.append(request)
      return (entries[request.url], delays[request.url], failures[request.url])
    }
    if let delay { try await Task.sleep(for: delay) }
    if let failure { throw MediaError.Fetch(detail: failure) }
    guard let entry else { throw MediaError.Fetch(detail: "offline") }
    let data: Data
    switch entry.content {
    case .bytes(let bytes): data = bytes
    case .file(let url):
      do { data = try Data(contentsOf: url) } catch { throw MediaError.Fetch(detail: "unreadable \(url.path)") }
    }
    var headers = ["Accept-Ranges": "bytes"]
    if let type = entry.contentType { headers["Content-Type"] = type }
    guard let start = request.rangeStart else {
      headers["Content-Length"] = String(data.count)
      return FetchResponse(status: 200, headers: headers, body: TestBody(data, chunkSize: chunkSize))
    }
    guard start < UInt64(data.count) else {
      headers["Content-Range"] = "bytes */\(data.count)"
      headers["Content-Length"] = "0"
      return FetchResponse(status: 416, headers: headers, body: TestBody(Data(), chunkSize: chunkSize))
    }
    let end = min(request.rangeEnd ?? UInt64.max, UInt64(data.count - 1))
    let slice = data.subdata(in: Int(start)..<Int(end) + 1)
    headers["Content-Range"] = "bytes \(start)-\(end)/\(data.count)"
    headers["Content-Length"] = String(slice.count)
    return FetchResponse(status: 206, headers: headers, body: TestBody(slice, chunkSize: chunkSize))
  }
}

private final class TestBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private let chunkSize: Int
  private var rest: Data

  init(_ data: Data, chunkSize: Int) {
    rest = data
    self.chunkSize = chunkSize
  }

  func nextChunk() async throws -> Data? {
    lock.withLock {
      let count = min(rest.count, chunkSize)
      guard count > 0 else { return nil }
      defer { rest.removeFirst(count) }
      return Data(rest.prefix(count))
    }
  }

  func release() {
    lock.withLock { rest = Data() }
  }
}

import Foundation
import MediaLayer

public final class URLSessionFetcher: Fetcher, @unchecked Sendable {
  public static let defaultBufferLimit = 1 << 20
  public static let defaultTimeout: TimeInterval = 30

  private let router = StreamRouter()
  private let session: URLSession
  private let bufferLimit: Int
  private let timeout: TimeInterval

  public init(timeout: TimeInterval = defaultTimeout, bufferLimit: Int = defaultBufferLimit) {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.urlCache = nil
    configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
    configuration.timeoutIntervalForRequest = timeout
    session = URLSession(configuration: configuration, delegate: router, delegateQueue: nil)
    self.timeout = timeout
    self.bufferLimit = bufferLimit
  }

  deinit {
    session.invalidateAndCancel()
  }

  public func fetch(request: FetchRequest) async throws -> FetchResponse {
    guard let url = URL(string: request.url) else {
      throw MediaError.Fetch(detail: "invalid url \(request.url)")
    }
    var urlRequest = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: timeout)
    for (name, value) in request.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    if let start = request.rangeStart {
      urlRequest.setValue("bytes=\(start)-\(request.rangeEnd.map(String.init) ?? "")", forHTTPHeaderField: "Range")
    }
    let task = session.dataTask(with: urlRequest)
    let body = StreamingBody(
      limit: bufferLimit,
      pause: { task.suspend() },
      resume: { task.resume() },
      cancel: { task.cancel() }
    )
    let response = try await withTaskCancellationHandler {
      try await router.start(task, body: body)
    } onCancel: {
      task.cancel()
    }
    return FetchResponse(status: UInt16(response.statusCode), headers: response.headers, body: body)
  }
}

private struct Head {
  let statusCode: Int
  let headers: [String: String]
}

private final class StreamRouter: NSObject, URLSessionDataDelegate, @unchecked Sendable {
  private struct Entry {
    weak var body: StreamingBody?
    var head: CheckedContinuation<Head, Error>?
  }

  private let lock = NSLock()
  private var entries: [Int: Entry] = [:]

  func start(_ task: URLSessionDataTask, body: StreamingBody) async throws -> Head {
    try await withCheckedThrowingContinuation { continuation in
      lock.lock()
      entries[task.taskIdentifier] = Entry(body: body, head: continuation)
      lock.unlock()
      task.resume()
    }
  }

  func urlSession(
    _ session: URLSession,
    dataTask: URLSessionDataTask,
    didReceive response: URLResponse,
    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
  ) {
    guard let http = response as? HTTPURLResponse else {
      completionHandler(.cancel)
      return
    }
    var headers: [String: String] = [:]
    for (name, value) in http.allHeaderFields { headers["\(name)"] = "\(value)" }
    lock.lock()
    let continuation = entries[dataTask.taskIdentifier]?.head
    entries[dataTask.taskIdentifier]?.head = nil
    lock.unlock()
    continuation?.resume(returning: Head(statusCode: http.statusCode, headers: headers))
    completionHandler(.allow)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    lock.lock()
    let body = entries[dataTask.taskIdentifier]?.body
    lock.unlock()
    body?.receive(data)
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    lock.lock()
    let entry = entries.removeValue(forKey: task.taskIdentifier)
    lock.unlock()
    let failure = error.map { MediaError.Fetch(detail: $0.localizedDescription) }
    entry?.head?.resume(throwing: failure ?? MediaError.Fetch(detail: "no response"))
    entry?.body?.finish(failure)
  }
}

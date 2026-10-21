import Foundation
import CoreGraphics
import ImageIO
import Network
import UniformTypeIdentifiers

final class LocalHTTPServer: @unchecked Sendable {
  static let payload = Data((0..<1000).map { UInt8($0 % 251) })

  let closed: AsyncStream<String>
  let requests: AsyncStream<String>
  private let requestsContinuation: AsyncStream<String>.Continuation
  private let closedContinuation: AsyncStream<String>.Continuation
  private let listener: NWListener
  private let queue = DispatchQueue(label: "local-http-server")
  private let lock = NSLock()
  private var gates: [CheckedContinuation<Void, Never>] = []
  private var gateOpen = false
  private(set) var port: UInt16 = 0
  private var assets: [String: Data] = [:]
  private var seen: [String: [String: String]] = [:]
  private var counts: [String: Int] = [:]

  init() throws {
    (closed, closedContinuation) = AsyncStream.makeStream()
    (requests, requestsContinuation) = AsyncStream.makeStream()
    listener = try NWListener(using: .tcp, on: .any)
  }

  func start() async throws {
    listener.newConnectionHandler = { [self] connection in handle(connection) }
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
      let resumed = NSLock()
      var done = false
      listener.stateUpdateHandler = { state in
        resumed.lock()
        defer { resumed.unlock() }
        guard !done else { return }
        switch state {
        case .ready: done = true; continuation.resume()
        case .failed(let error): done = true; continuation.resume(throwing: error)
        default: break
        }
      }
      listener.start(queue: queue)
    }
    port = listener.port?.rawValue ?? 0
  }

  func stop() { listener.cancel() }

  func serve(_ data: Data, at path: String) { lock.withLock { assets[path] = data } }

  func requestCount(of path: String) -> Int { lock.withLock { counts[path] ?? 0 } }

  func headers(of path: String) -> [String: String] { lock.withLock { seen[path] ?? [:] } }

  func url(_ path: String) -> String { "http://127.0.0.1:\(port)\(path)" }

  func openGate() {
    let waiting = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      gateOpen = true
      defer { gates = [] }
      return gates
    }
    waiting.forEach { $0.resume() }
  }

  private func waitGate() async {
    await withCheckedContinuation { continuation in
      let open = lock.withLock { () -> Bool in
        if !gateOpen { gates.append(continuation) }
        return gateOpen
      }
      if open { continuation.resume() }
    }
  }

  private func handle(_ connection: NWConnection) {
    connection.start(queue: queue)
    readRequest(connection, buffer: Data())
  }

  private func readRequest(_ connection: NWConnection, buffer: Data) {
    connection.receive(minimumIncompleteLength: 1, maximumLength: 8192) { [self] data, _, complete, error in
      var buffer = buffer
      if let data { buffer.append(data) }
      if let end = buffer.range(of: Data("\r\n\r\n".utf8)) {
        let head = String(decoding: buffer[..<end.lowerBound], as: UTF8.self)
        Task { await respond(connection, head: head) }
      } else if complete || error != nil {
        connection.cancel()
      } else {
        readRequest(connection, buffer: buffer)
      }
    }
  }

  private func respond(_ connection: NWConnection, head: String) async {
    let lines = head.components(separatedBy: "\r\n")
    let path = lines[0].split(separator: " ").dropFirst().first.map(String.init) ?? "/"
    var headers: [String: String] = [:]
    for line in lines.dropFirst() {
      guard let colon = line.firstIndex(of: ":") else { continue }
      headers[line[..<colon].lowercased()] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
    }
    lock.withLock {
      seen[path] = headers
      counts[path, default: 0] += 1
    }
    requestsContinuation.yield(path)
    if let asset = lock.withLock({ assets[path] }) {
      await send(connection, status: "200 OK", extra: ["Content-Type: application/octet-stream"], body: asset)
      return
    }
    switch path {
    case "/bytes": await sendBytes(connection, range: headers["range"])
    case "/headers":
      let text = headers.sorted { $0.key < $1.key }.map { "\($0.key): \($0.value)" }.joined(separator: "\n")
      await send(connection, status: "200 OK", extra: [], body: Data(text.utf8))
    case "/gated":
      let half = Self.payload.prefix(500)
      await write(connection, Data("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1000\r\n\r\n".utf8) + half)
      await waitGate()
      await write(connection, Self.payload.suffix(500))
      connection.cancel()
    case "/endless":
      await write(connection, Data("HTTP/1.1 200 OK\r\nContent-Length: 100000000\r\n\r\n".utf8))
      let chunk = Data(repeating: 7, count: 4096)
      while await write(connection, chunk) {}
      closedContinuation.yield(path)
    case "/hang":
      watchClose(connection, path: path)
    default:
      await send(connection, status: "404 Not Found", extra: [], body: Data())
    }
  }

  private func sendBytes(_ connection: NWConnection, range: String?) async {
    guard let range, range.hasPrefix("bytes=") else {
      await send(connection, status: "200 OK", extra: [], body: Self.payload)
      return
    }
    let parts = range.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
    let start = Int(parts[0]) ?? 0
    let end = min(Int(parts[1]) ?? Self.payload.count - 1, Self.payload.count - 1)
    await send(
      connection,
      status: "206 Partial Content",
      extra: ["Content-Range: bytes \(start)-\(end)/\(Self.payload.count)"],
      body: Self.payload[start...end]
    )
  }

  private func send(_ connection: NWConnection, status: String, extra: [String], body: Data) async {
    let head = (["HTTP/1.1 \(status)", "Content-Length: \(body.count)", "Connection: close"] + extra).joined(separator: "\r\n")
    await write(connection, Data((head + "\r\n\r\n").utf8) + body)
    connection.cancel()
  }

  @discardableResult
  private func write(_ connection: NWConnection, _ data: Data) async -> Bool {
    await withCheckedContinuation { continuation in
      connection.send(content: data, completion: .contentProcessed { continuation.resume(returning: $0 == nil) })
    }
  }

  private func watchClose(_ connection: NWConnection, path: String) {
    connection.receive(minimumIncompleteLength: 1, maximumLength: 1) { [self] _, _, complete, error in
      if complete || error != nil { closedContinuation.yield(path) } else { watchClose(connection, path: path) }
    }
  }
}

func pngData(side: Int, height: Int? = nil) -> Data {
  let context = CGContext(
    data: nil, width: side, height: height ?? side, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  )!
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: side, height: height ?? side))
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil)!
  CGImageDestinationAddImage(destination, context.makeImage()!, nil)
  CGImageDestinationFinalize(destination)
  return data as Data
}


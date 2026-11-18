import Foundation
import MediaLayer
import MediaLayerTesting
import Observation
import Testing
@testable import MediaLayerUI

final class SwitchMicrophone: MicrophonePermissionPort, @unchecked Sendable {
  private let lock = NSLock()
  private var value: MicrophonePermission
  private var sink: (@Sendable (MicrophonePermission) -> Void)?

  init(granted: Bool = true) { value = granted ? .granted : .denied }
  init(state: MicrophonePermission) { value = state }

  func state() -> MicrophonePermission { lock.withLock { value } }

  func changes(_ sink: @escaping @Sendable (MicrophonePermission) -> Void) -> @Sendable () -> Void {
    lock.withLock { self.sink = sink }
    return { [weak self] in self?.lock.withLock { self?.sink = nil } }
  }

  func set(_ granted: Bool) { set(granted ? .granted : .denied) }

  func set(_ state: MicrophonePermission) {
    let sink = lock.withLock { () -> (@Sendable (MicrophonePermission) -> Void)? in
      value = state
      return self.sink
    }
    sink?(state)
  }

  /// A change with no push: the system answered a prompt and said nothing.
  func setSilently(_ state: MicrophonePermission) { lock.withLock { value = state } }
}

/// A layer opened the way the app opens it. Only the system ports are fakes.
final class AppLayer: @unchecked Sendable {
  let media: MediaLayer
  let session: FakeSessionPort
  let calls: FakeCallObserver
  let microphone: SwitchMicrophone

  private init(media: MediaLayer, session: FakeSessionPort, calls: FakeCallObserver, microphone: SwitchMicrophone) {
    self.media = media
    self.session = session
    self.calls = calls
    self.microphone = microphone
  }

  static func open(
    session: FakeSessionPort = FakeSessionPort(),
    microphone: SwitchMicrophone = SwitchMicrophone(),
    slots: (any PlayerSlotFactory)? = nil
  ) async throws -> AppLayer {
    let calls = FakeCallObserver()
    let media = try await MediaLayer.open(
      defaults: { SpecDefaults(defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: []) },
      fetcher: TestFetcher(),
      settings: QuietSettings(),
      directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
      audioFocus: .mix,
      imageMemoryBytes: 1_000_000,
      onSimulator: false,
      ports: { _ in AudioPorts(session: session, calls: calls, permission: microphone) },
      build: { config in
        var config = config
        if let slots { config.slots = slots }
        return try MediaLayer.open(config: config)
      }
    )
    return AppLayer(media: media, session: session, calls: calls, microphone: microphone)
  }

  func close() async { await media.awaitClosed() }
}

/// Every value of a public stream, in order, and whether the stream ended.
final class Collected<Value: Sendable>: @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [Value] = []
  private var ended = false
  private var task: Task<Void, Never>?

  init(_ stream: AsyncStream<Value>) {
    task = Task { [weak self] in
      for await value in stream {
        self?.lock.withLock { self?.seen.append(value) }
        TestSignal.shared.fire()
      }
      self?.lock.withLock { self?.ended = true }
      TestSignal.shared.fire()
    }
  }

  var values: [Value] { lock.withLock { seen } }
  var last: Value? { lock.withLock { seen.last } }
  var finished: Bool { lock.withLock { ended } }

  deinit { task?.cancel() }
}

/// Wakes `eventually` on every change of the observed public properties.
@MainActor
final class Watched {
  private let read: @MainActor () -> Void
  private var live = true

  init(_ read: @escaping @MainActor () -> Void) {
    self.read = read
    track()
  }

  private func track() {
    guard live else { return }
    withObservationTracking { read() } onChange: {
      Task { @MainActor [weak self] in
        TestSignal.shared.fire()
        self?.track()
      }
    }
  }

  func stop() { live = false }
}

func appFixture(_ name: String, _ ext: String) -> String {
  Bundle.module.url(forResource: name, withExtension: ext, subdirectory: "Fixtures")!.absoluteString
}

func temporaryFile(_ ext: String) -> URL {
  FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).\(ext)")
}

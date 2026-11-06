import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class CountingCapture: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private var started = 0
  private var file: URL?

  var starts: Int { lock.withLock { started } }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    lock.withLock {
      started += 1
      self.file = file
    }
    try Data([1, 2, 3]).write(to: file)
  }

  func peak() -> Float { 0.5 }

  func stop() throws {}
}

private final class Ticks: @unchecked Sendable {
  private let lock = NSLock()
  private var value: UInt64 = 1000
  func next() -> UInt64 { lock.withLock { value += 500; return value } }
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct CallHoldRecordingTests {
  private let ticks = Ticks()

  private func file() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
  }

  private func makeRecorder(_ capture: VoiceCapture, gate: CallGate) -> MediaVoiceRecorder {
    MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, audio: { nil }, clock: { ticks.next() }, levelInterval: .milliseconds(5), calls: gate)
  }

  @Test
  func inv_call_09_a_call_stops_a_running_recording_with_the_in_call_refusal_and_deletes_the_file() async throws {
    let gate = CallGate()
    let recorder = makeRecorder(CountingCapture(), gate: gate)
    let target = file()
    try await recorder.start(to: target)
    #expect(recorder.isRecording)

    gate.hold()
    await eventually("stopped") { !recorder.isRecording }
    do {
      _ = try await recorder.stop()
      Issue.record("a recording survived the call")
    } catch let MediaError.Voice(detail) {
      #expect(detail == voiceRefusalText(reason: .inCall))
    }
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_call_10_a_recording_started_during_a_call_is_refused_before_the_microphone() async throws {
    let gate = CallGate()
    let capture = CountingCapture()
    let recorder = makeRecorder(capture, gate: gate)
    gate.hold()

    do {
      try await recorder.start(to: file())
      Issue.record("a recording started during a call")
    } catch let MediaError.Voice(detail) {
      #expect(detail == voiceRefusalText(reason: .inCall))
    }
    #expect(capture.starts == 0)
    #expect(!recorder.isRecording)
  }

  @Test
  func inv_call_11_a_recording_is_allowed_after_the_last_hold_is_released() async throws {
    let gate = CallGate()
    let recorder = makeRecorder(CountingCapture(), gate: gate)
    gate.hold()
    gate.hold()
    gate.release()
    #expect(gate.inCall)
    gate.release()

    try await recorder.start(to: file())

    #expect(recorder.isRecording)
    _ = try await recorder.stop()
  }
}

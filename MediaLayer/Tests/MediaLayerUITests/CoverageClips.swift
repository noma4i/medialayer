#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import MediaLayer
@testable import MediaLayerUI

/// Real mp4 files written by the real `CameraTake`: the PCM that reaches the writer is chosen by the test, the video always covers the whole fixture.
enum CoverageClips {
  typealias Fixture = CameraTakeTests.Fixture

  /// `audio` are the indexes of the 10 ms PCM buffers that the writer gets; the 6 video frames last 200 ms.
  static func write(audio indexes: [Int]) async throws -> URL {
    try await writeJudged(audio: indexes).url
  }

  /// The file and the verdict the writer gave with its outcome, from the PCM it was handed.
  static func writeJudged(audio indexes: [Int]) async throws -> (url: URL, coverage: AudioCoverage) {
    let fixture = try Fixture.make()
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("coverage-\(UUID().uuidString).mp4")
    let (stream, events) = AsyncStream<String>.makeStream()
    let judged = JudgedBox()
    let take = CameraTake(
      url: url, generation: Fixture.generation, settings: fixture.settings,
      onBegin: { _, _ in events.yield("begin") },
      onStarted: { events.yield("started") },
      onOutcome: { outcome in
        if case let .finished(_, _, _, coverage) = outcome {
          judged.set(coverage)
          events.yield("finished")
        } else { events.yield("failed \(outcome)") }
      }
    )
    var iterator = stream.makeAsyncIterator()
    for index in indexes { take.appendAudio(fixture.audio[index], generation: Fixture.generation) }
    guard await iterator.next() == "begin" else { throw NSError(domain: "coverage", code: 1) }
    take.confirmBegin(generation: Fixture.generation)
    guard await iterator.next() == "started" else { throw NSError(domain: "coverage", code: 2) }
    for buffer in fixture.video { take.appendVideo(buffer, generation: Fixture.generation) }
    take.stop()
    take.audioEnded()
    guard await iterator.next() == "finished" else { throw NSError(domain: "coverage", code: 3) }
    guard let coverage = judged.value else { throw NSError(domain: "coverage", code: 4) }
    return (url, coverage)
  }

  private final class JudgedBox: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: AudioCoverage?
    var value: AudioCoverage? { lock.withLock { stored } }
    func set(_ coverage: AudioCoverage) { lock.withLock { stored = coverage } }
  }

  static let whole = Array(0..<Fixture.audioCount)
  static let hole = Array(0..<6) + Array(12..<Fixture.audioCount)
  static let cut = Array(0..<8)
}
#endif

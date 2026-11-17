#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
@testable import MediaLayerUI

/// Real mp4 files written by the real `CameraTake`: the PCM that reaches the writer is chosen by the test, the video always covers the whole fixture.
enum CoverageClips {
  typealias Fixture = CameraTakeTests.Fixture

  /// `audio` are the indexes of the 10 ms PCM buffers that the writer gets; the 6 video frames last 200 ms.
  static func write(audio indexes: [Int]) async throws -> URL {
    let fixture = try Fixture.make()
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("coverage-\(UUID().uuidString).mp4")
    let (stream, events) = AsyncStream<String>.makeStream()
    let take = CameraTake(
      url: url, generation: Fixture.generation, settings: fixture.settings,
      onBegin: { _, _ in events.yield("begin") },
      onStarted: { events.yield("started") },
      onOutcome: { outcome in
        if case .finished = outcome { events.yield("finished") } else { events.yield("failed \(outcome)") }
      }
    )
    var iterator = stream.makeAsyncIterator()
    for index in indexes { take.appendAudio(fixture.audio[index], generation: Fixture.generation) }
    guard await iterator.next() == "begin" else { throw NSError(domain: "coverage", code: 1) }
    take.confirmBegin(generation: Fixture.generation)
    guard await iterator.next() == "started" else { throw NSError(domain: "coverage", code: 2) }
    for buffer in fixture.video { take.appendVideo(buffer, generation: Fixture.generation) }
    take.stop()
    guard await iterator.next() == "finished" else { throw NSError(domain: "coverage", code: 3) }
    return url
  }

  static let whole = Array(0..<Fixture.audioCount)
  static let hole = Array(0..<6) + Array(12..<Fixture.audioCount)
  static let cut = Array(0..<8)
}
#endif

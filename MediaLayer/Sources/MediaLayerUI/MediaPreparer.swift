@preconcurrency import AVFoundation
import Foundation
import MediaLayer

/// Prepares a file for sending by the descriptor of the app. The core decides (`mediaPrepareDecide`), this class reads the source facts and writes the plan.
public final class MediaPreparer: @unchecked Sendable {
  private let running = PrepareSerial([UInt64: PrepareRunning]())

  public init() {}

  /// The facts of the file: the planned result when `descriptor` is given, the source turned upright otherwise.
  public func probe(path: String, descriptor: Data?, mime: String?) async -> MediaProbeOutcome {
    let file = URL(fileURLWithPath: path)
    let probe = await PrepareFacts.read(file, mime: mime)
    switch probe {
    case let .failed(message):
      return .failed(message: message)
    default:
      guard let facts = mediaPrepareFacts(descriptor: descriptor, probe: probe) else {
        return .failed(message: "\(path) does not fit the prepare descriptor")
      }
      return .file(width: facts.width, height: facts.height, mime: facts.mime)
    }
  }

  /// One run: probe, decide, then pass, write or refuse. `progress` gets 0 to 1 and never goes back. A cancelled run removes its temp file and returns `.cancelled`.
  public func run(job: MediaPrepareJob, progress: @escaping @Sendable (Double) -> Void) async -> MediaPrepareOutcome {
    let entry = PrepareRunning()
    running.update { $0[job.attemptId] = entry }
    defer { running.update { $0[job.attemptId] = nil } }
    let work = Task.detached { () -> MediaPrepareOutcome in
      let destination = URL(fileURLWithPath: job.destination)
      let temp = destination.deletingLastPathComponent().appendingPathComponent(".\(UUID().uuidString).preparing")
      do {
        let probe = await PrepareFacts.read(URL(fileURLWithPath: job.source), mime: nil)
        try Task.checkCancellation()
        switch mediaPrepareDecide(descriptor: job.descriptor, probe: probe) {
        case .pass:
          progress(1)
          return .unchanged(name: nil, mime: nil)
        case let .refuse(reason, message):
          return reason == .cannotDecode ? .cannotDecode(message: message) : .failed(message: message)
        case let .run(plan):
          let monotonic = PrepareProgress(progress)
          let mime: String
          switch plan {
          case let .video(plan, _, audioBitrate, audioChannels, audioSampleRate, mirror):
            let audio = PrepareAudio(bitrate: audioBitrate, channels: audioChannels, sampleRate: audioSampleRate)
            try await PrepareVideoWrite.run(source: URL(fileURLWithPath: job.source), output: temp, plan: plan, audio: audio, mirror: mirror, entry: entry, report: monotonic.report)
            mime = "video/mp4"
          case let .image(plan, quality, format):
            try PrepareImageWrite.run(source: URL(fileURLWithPath: job.source), output: temp, plan: plan, quality: quality, format: format)
            mime = format == .png ? "image/png" : "image/jpeg"
          }
          try Task.checkCancellation()
          _ = try FileManager.default.replaceItemAt(destination, withItemAt: temp)
          monotonic.report(1)
          return .done(name: nil, mime: mime)
        }
      } catch {
        try? FileManager.default.removeItem(at: temp)
        return error is CancellationError ? .cancelled : .failed(message: error.localizedDescription)
      }
    }
    entry.hold(work)
    return await work.value
  }

  /// Stops that run at once. Its temp file is removed and the run returns `.cancelled`.
  public func cancel(attemptId: UInt64) {
    running.update { $0[attemptId] }?.cancel()
  }
}

private final class PrepareProgress: @unchecked Sendable {
  private let state = PrepareSerial(0.0)
  private let sink: @Sendable (Double) -> Void

  init(_ sink: @escaping @Sendable (Double) -> Void) { self.sink = sink }

  func report(_ value: Double) {
    let next = min(max(value, 0), 1)
    if state.update({ last -> Bool in
      guard next > last else { return false }
      last = next
      return true
    }) { sink(next) }
  }
}

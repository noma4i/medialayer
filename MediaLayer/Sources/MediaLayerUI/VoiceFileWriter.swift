@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

/// The native file boundary of a voice take: `open` makes the file with its first sample.
protocol VoiceFileWriter: AnyObject, Sendable {
  var isReady: Bool { get }
  /// Set once the native writer has failed: nothing more will be taken.
  var failure: Error? { get }
  func open(first: CMSampleBuffer, settings: [String: Any]) throws
  func append(_ buffer: CMSampleBuffer) throws
  func finish() async throws
  func cancel()
}

extension VoiceFileWriter {
  var failure: Error? { nil }
}

final class AssetVoiceWriter: VoiceFileWriter, @unchecked Sendable {
  private let url: URL
  private var writer: AVAssetWriter?
  private var input: AVAssetWriterInput?

  init(url: URL) { self.url = url }

  var isReady: Bool { input?.isReadyForMoreMediaData ?? false }
  var failure: Error? { writer?.status == .failed ? (writer?.error ?? VoiceTake.Failure.appendFailed) : nil }

  func open(first: CMSampleBuffer, settings: [String: Any]) throws {
    if FileManager.default.fileExists(atPath: url.path) { NativeFailures.removeFile(url) }
    let made = try AVAssetWriter(outputURL: url, fileType: .m4a)
    let audio = AVAssetWriterInput(mediaType: .audio, outputSettings: settings, sourceFormatHint: CMSampleBufferGetFormatDescription(first))
    audio.expectsMediaDataInRealTime = true
    guard made.canAdd(audio) else { throw VoiceTake.Failure.writerRejected }
    made.add(audio)
    writer = made
    input = audio
    guard made.startWriting() else { throw made.error ?? VoiceTake.Failure.writerRejected }
    made.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(first))
  }

  func append(_ buffer: CMSampleBuffer) throws {
    guard let input, let writer else { throw VoiceTake.Failure.writerRejected }
    guard input.append(buffer) else { throw writer.error ?? VoiceTake.Failure.appendFailed }
  }

  func finish() async throws {
    guard let writer, let input else { return }
    input.markAsFinished()
    await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in writer.finishWriting { done.resume() } }
    guard writer.status == .completed else { throw writer.error ?? VoiceTake.Failure.appendFailed }
  }

  func cancel() {
    if let writer, writer.status == .writing { writer.cancelWriting() }
  }
}

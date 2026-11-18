#if os(iOS)
import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import Testing
@testable import MediaLayerUI

@Suite(.timeLimit(.minutes(1)))
struct CameraTakeTests {
  private enum Event: Sendable {
    case begin(generation: UInt64, pts: CMTime, fileExisted: Bool)
    case started
    case outcome(CameraTake.Outcome)
  }

  struct Fixture {
    static let generation: UInt64 = 7
    static let sampleRate = 48_000.0
    static let framesPerAudio = 480
    static let audioCount = 20
    static let videoCount = 6
    static let width = 320
    static let height = 240
    static let arm = CMTime(seconds: 100, preferredTimescale: 48_000)

    let audio: [CMSampleBuffer]
    let video: [CMSampleBuffer]
    let stale: [CMSampleBuffer]

    var settings: CameraTake.Settings {
      let bytes = (audio + stale).reduce(0) { $0 + CMSampleBufferGetTotalSampleSize($1) + CameraTake.entryCost }
        + video.reduce(0) { $0 + CVPixelBufferGetDataSize(CMSampleBufferGetImageBuffer($1)!) + CameraTake.entryCost }
      return CameraTake.Settings(
        video: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: Self.width, AVVideoHeightKey: Self.height],
        audio: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVNumberOfChannelsKey: 1, AVSampleRateKey: Self.sampleRate],
        transform: .identity, maxDurationMs: 0, fileType: .mp4, fragmentInterval: CMTime(seconds: 10, preferredTimescale: 600),
        armPTS: Self.arm, maxPendingAudio: audio.count + stale.count, maxPendingVideo: video.count,
        maxHeldAudio: audio.count, memoryBudgetBytes: bytes
      )
    }

    static func make() throws -> Fixture {
      let step = CMTime(value: CMTimeValue(framesPerAudio), timescale: CMTimeScale(sampleRate))
      let audio = try (0..<audioCount).map { try audioBuffer(at: arm + CMTimeMultiply(step, multiplier: Int32($0))) }
      let frame = CMTime(value: 1, timescale: 30)
      let video = try (0..<videoCount).map { try videoBuffer(at: arm + CMTimeMultiply(frame, multiplier: Int32($0))) }
      let stale = [try audioBuffer(at: arm - CMTime(seconds: 1, preferredTimescale: 48_000))]
      return Fixture(audio: audio, video: video, stale: stale)
    }

    private static func audioBuffer(at pts: CMTime) throws -> CMSampleBuffer {
      var description = AudioStreamBasicDescription(
        mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM, mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
        mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0
      )
      var format: CMAudioFormatDescription?
      try check(CMAudioFormatDescriptionCreate(allocator: nil, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format))
      let length = framesPerAudio * 2
      var block: CMBlockBuffer?
      try check(CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: length, blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: length, flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block))
      try check(CMBlockBufferFillDataBytes(with: 0, blockBuffer: block!, offsetIntoDestination: 0, dataLength: length))
      var buffer: CMSampleBuffer?
      try check(CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: nil, dataBuffer: block!, formatDescription: format!, sampleCount: framesPerAudio, presentationTimeStamp: pts, packetDescriptions: nil, sampleBufferOut: &buffer))
      return buffer!
    }

    private static func videoBuffer(at pts: CMTime) throws -> CMSampleBuffer {
      var pixels: CVPixelBuffer?
      try check(CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA, nil, &pixels))
      var format: CMVideoFormatDescription?
      try check(CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: pixels!, formatDescriptionOut: &format))
      var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30), presentationTimeStamp: pts, decodeTimeStamp: .invalid)
      var buffer: CMSampleBuffer?
      try check(CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: pixels!, formatDescription: format!, sampleTiming: &timing, sampleBufferOut: &buffer))
      return buffer!
    }

    private static func check(_ status: OSStatus) throws {
      guard status == noErr else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
    }
  }

  private final class TakeBox: @unchecked Sendable {
    var take: CameraTake?
  }

  private func makeTake(
    _ fixture: Fixture, settings: CameraTake.Settings? = nil, whenStarted: (@Sendable (CameraTake) -> Void)? = nil
  ) -> (CameraTake, AsyncStream<Event>.Iterator, URL) {
    let box = TakeBox()
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("camera-take-\(UUID().uuidString).mp4")
    let (stream, events) = AsyncStream<Event>.makeStream()
    let take = CameraTake(
      url: url, generation: Fixture.generation, settings: settings ?? fixture.settings,
      onBegin: { generation, pts in events.yield(.begin(generation: generation, pts: pts, fileExisted: FileManager.default.fileExists(atPath: url.path))) },
      onStarted: { events.yield(.started); if let take = box.take { whenStarted?(take) } },
      onOutcome: { events.yield(.outcome($0)) }
    )
    box.take = take
    return (take, stream.makeAsyncIterator(), url)
  }

  @Test
  func inv_capture_44_a_take_writes_both_streams_after_a_fresh_pcm_and_the_confirm() async throws {
    let fixture = try Fixture.make()
    var (take, events, url) = makeTake(fixture)
    defer { try? FileManager.default.removeItem(at: url) }
    take.appendAudio(fixture.stale[0], generation: Fixture.generation)
    take.appendAudio(fixture.audio[0], generation: Fixture.generation + 1)
    for buffer in fixture.audio { take.appendAudio(buffer, generation: Fixture.generation) }
    guard case let .begin(generation, pts, fileExisted)? = await events.next() else { Issue.record("no begin first"); return }
    #expect(generation == Fixture.generation)
    #expect(pts == CMSampleBufferGetPresentationTimeStamp(fixture.audio[0]))
    #expect(!fileExisted)
    take.confirmBegin(generation: Fixture.generation)
    guard case .started? = await events.next() else { Issue.record("no started after confirm"); return }
    for buffer in fixture.video { take.appendVideo(buffer, generation: Fixture.generation) }
    take.stop()
    guard case let .outcome(.finished(finished, _, reachedLimit))? = await events.next() else { Issue.record("no finished outcome"); return }
    #expect(finished == url)
    #expect(!reachedLimit)
    let facts = try await scanCaptureFacts(at: url)
    guard case let .read(video) = facts.video, case let .read(audio) = facts.audio else { Issue.record("a stream is missing: \(facts)"); return }
    #expect(video.samples > 0)
    #expect(audio.samples > 0)
  }

  @Test
  func inv_capture_51_the_take_ends_with_the_last_video_frame_and_the_sound_is_cut_there() async throws {
    let fixture = try Fixture.make()
    var (take, events, url) = makeTake(fixture)
    defer { try? FileManager.default.removeItem(at: url) }
    for buffer in fixture.audio { take.appendAudio(buffer, generation: Fixture.generation) }
    guard case .begin? = await events.next() else { Issue.record("no begin first"); return }
    take.confirmBegin(generation: Fixture.generation)
    guard case .started? = await events.next() else { Issue.record("no started after confirm"); return }
    let shown = fixture.video.prefix(3)
    for buffer in shown { take.appendVideo(buffer, generation: Fixture.generation) }
    take.stop()
    guard case let .outcome(.finished(_, durationMs, _))? = await events.next() else { Issue.record("no finished outcome"); return }
    #expect(durationMs == 100)
    let asset = AVURLAsset(url: url)
    let audio = try #require(try await asset.loadTracks(withMediaType: .audio).first)
    let video = try #require(try await asset.loadTracks(withMediaType: .video).first)
    let audioEnd = try await audio.load(.timeRange).end.seconds
    let videoEnd = try await video.load(.timeRange).end.seconds
    #expect(abs(videoEnd - 0.1) < 0.02)
    #expect(abs(audioEnd - videoEnd) < 0.03)
  }

  @Test
  func inv_capture_51_a_frame_ends_at_its_duration_or_the_gap_before_it() {
    let scale: CMTimeScale = 600
    let pts = CMTime(value: 60, timescale: scale)
    let step = CMTime(value: 20, timescale: scale)
    #expect(CameraTake.frameEnd(pts: pts, duration: step, previous: .invalid) == pts + step)
    #expect(CameraTake.frameEnd(pts: pts, duration: .invalid, previous: pts - step) == pts + step)
    #expect(CameraTake.frameEnd(pts: pts, duration: .zero, previous: .invalid) == pts)
  }

  @Test
  func inv_capture_45_a_cancel_during_finish_ends_with_the_cancel_and_leaves_no_file() async throws {
    let fixture = try Fixture.make()
    nonisolated(unsafe) let video = fixture.video
    var (take, events, url) = makeTake(fixture) { take in
      for buffer in video { take.appendVideo(buffer, generation: Fixture.generation) }
      take.stop()
      take.cancel()
    }
    defer { try? FileManager.default.removeItem(at: url) }
    for buffer in fixture.audio { take.appendAudio(buffer, generation: Fixture.generation) }
    guard case .begin? = await events.next() else { Issue.record("no begin"); return }
    take.confirmBegin(generation: Fixture.generation)
    guard case .started? = await events.next() else { Issue.record("no started"); return }
    guard case .outcome(.cancelled)? = await events.next() else { Issue.record("the outcome is not the cancel"); return }
    #expect(!FileManager.default.fileExists(atPath: url.path))
  }

  @Test(arguments: [(pending: 0, bytesShort: 0, failure: CameraTake.Failure.audioBacklog), (pending: 1, bytesShort: 1, failure: .memoryBudget)])
  func inv_capture_46_an_own_audio_the_take_cannot_hold_ends_the_take_with_a_failure_and_no_file(pending: Int, bytesShort: Int, failure: CameraTake.Failure) async throws {
    let fixture = try Fixture.make()
    var settings = fixture.settings
    settings.maxPendingAudio = pending
    settings.memoryBudgetBytes = CMSampleBufferGetTotalSampleSize(fixture.audio[0]) + CameraTake.entryCost - bytesShort
    var (take, events, url) = makeTake(fixture, settings: settings)
    defer { try? FileManager.default.removeItem(at: url) }
    take.appendAudio(fixture.audio[0], generation: Fixture.generation)
    guard case let .outcome(.failed(error))? = await events.next() else { Issue.record("no failure outcome"); return }
    #expect(error as? CameraTake.Failure == failure)
    #expect(!FileManager.default.fileExists(atPath: url.path))
  }

  @Test
  func inv_capture_47_a_profile_without_audio_starts_on_the_first_video_without_a_begin() async throws {
    let fixture = try Fixture.make()
    var settings = fixture.settings
    settings.audio = nil
    var (take, events, url) = makeTake(fixture, settings: settings)
    defer { try? FileManager.default.removeItem(at: url) }
    for buffer in fixture.video { take.appendVideo(buffer, generation: Fixture.generation) }
    guard case .started? = await events.next() else { Issue.record("the first event is not started"); return }
    take.stop()
    let last = await events.next()
    guard case .outcome(.finished)? = last else { Issue.record("no finished outcome: \(String(describing: last))"); return }
    let facts = try await scanCaptureFacts(at: url)
    #expect(facts.audio == .absent)
    guard case let .read(video) = facts.video else { Issue.record("no video stream: \(facts)"); return }
    #expect(video.samples > 0)
  }
}
#endif

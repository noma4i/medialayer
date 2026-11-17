#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import Testing
@testable import MediaLayer
@testable import MediaLayerUI

@Suite(.timeLimit(.minutes(1)))
struct CameraTakeRelayTests {
  private typealias Fixture = CameraTakeTests.Fixture

  enum Scenario: CaseIterable {
    case audioAndVideo, videoOnly, limit, revokedTicket, cancelledMidTake
  }

  private final class Rig: @unchecked Sendable {
    let queue = DispatchQueue(label: "camera-relay-test")
    let relay: CameraTakeRelay
    private let sink: AsyncStream<CaptureEvent>.Continuation
    private var events: AsyncStream<CaptureEvent>.Iterator
    private var refusals: AsyncStream<RecordingRefusal>.Iterator
    private let refuse: AsyncStream<RecordingRefusal>.Continuation
    private(set) var seen: [CaptureEvent] = []

    init() {
      let (stream, sink) = AsyncStream<CaptureEvent>.makeStream()
      let (refusalStream, refuse) = AsyncStream<RecordingRefusal>.makeStream()
      self.sink = sink
      self.refuse = refuse
      events = stream.makeAsyncIterator()
      refusals = refusalStream.makeAsyncIterator()
      relay = CameraTakeRelay(queue: queue, emit: { sink.yield($0) }, idle: {})
    }

    func begin(_ fixture: Fixture, url: URL, ticket: RecordingTicket? = nil, audio: Bool = true, maxDurationMs: UInt64 = 0) {
      var settings = fixture.settings
      settings.maxDurationMs = maxDurationMs
      if !audio { settings.audio = nil }
      let refuse = refuse
      relay.begin(url: url, settings: settings, ticket: ticket) { refuse.yield($0) }
    }

    func open(_ fixture: Fixture, audio: Bool = true) {
      relay.appendAudio(fixture.stale[0])
      if audio { relay.appendAudio(fixture.audio[0]) } else { relay.appendVideo(fixture.video[0]) }
    }

    func flow(_ fixture: Fixture, audio: Bool = true) {
      let sound = audio ? fixture.audio.dropFirst().map { (buffer: $0, isAudio: true) } : []
      let frames = fixture.video.dropFirst(audio ? 0 : 1).map { (buffer: $0, isAudio: false) }
      let ordered = (sound + frames).enumerated().sorted {
        let (left, right) = (CMSampleBufferGetPresentationTimeStamp($0.element.buffer), CMSampleBufferGetPresentationTimeStamp($1.element.buffer))
        return left == right ? $0.offset < $1.offset : left < right
      }
      for (_, item) in ordered {
        if item.isAudio { relay.appendAudio(item.buffer) } else { relay.appendVideo(item.buffer) }
      }
    }

    func next() async -> CaptureEvent? {
      guard let event = await events.next() else { return nil }
      seen.append(event)
      return event
    }

    func nextRefusal() async -> RecordingRefusal? { await refusals.next() }

    func settle() async -> [CaptureEvent] {
      queue.sync {}
      sink.finish()
      while await next() != nil {}
      return seen
    }
  }

  private func decodedRange(of url: URL) async throws -> (start: CMTime, end: CMTime)? {
    let asset = AVURLAsset(url: url)
    guard let track = try await asset.loadTracks(withMediaType: .audio).first else { return nil }
    let reader = try AVAssetReader(asset: asset)
    let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
      AVFormatIDKey: kAudioFormatLinearPCM, AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
      AVNumberOfChannelsKey: 1, AVSampleRateKey: Fixture.sampleRate
    ])
    reader.add(output)
    guard reader.startReading() else { return nil }
    var start = CMTime.invalid
    var frames = 0
    while let buffer = output.copyNextSampleBuffer() {
      if !start.isValid { start = CMSampleBufferGetPresentationTimeStamp(buffer) }
      frames += CMSampleBufferGetNumSamples(buffer)
    }
    return (start, start + CMTime(value: CMTimeValue(frames), timescale: CMTimeScale(Fixture.sampleRate)))
  }

  private func url() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("camera-relay-\(UUID().uuidString).mp4")
  }

  private func record(_ rig: Rig, _ fixture: Fixture, url: URL, ticket: RecordingTicket? = nil, audio: Bool = true, maxDurationMs: UInt64 = 0, stop: Bool = true) async {
    rig.begin(fixture, url: url, ticket: ticket, audio: audio, maxDurationMs: maxDurationMs)
    #expect(!FileManager.default.fileExists(atPath: url.path))
    rig.open(fixture, audio: audio)
    if audio, await rig.next() != .audioFlow(flowing: true) { Issue.record("no flow of the first pcm"); return }
    guard case .recordStarted? = await rig.next() else { Issue.record("no started"); return }
    rig.flow(fixture, audio: audio)
    if stop { rig.relay.stop() }
    _ = await rig.next()
  }

  @Test(arguments: Scenario.allCases)
  func inv_capture_48_a_take_through_the_relay_gives_each_event_once_and_only_the_live_take_ends(_ scenario: Scenario) async throws {
    let fixture = try Fixture.make()
    let audio = try AudioRig()
    guard case let .success(ticket) = await audio.link.admit(.video) else { Issue.record("no ticket"); return }
    defer { ticket.end() }
    let rig = Rig()
    let first = url()
    let last = url()
    defer { [first, last].forEach { try? FileManager.default.removeItem(at: $0) } }
    let flow = CaptureEvent.audioFlow(flowing: true)
    var expected: [CaptureEvent] = [flow, .recordStarted, .recordFinished(end: .completed)]
    switch scenario {
    case .audioAndVideo:
      await record(rig, fixture, url: last, ticket: ticket)
    case .videoOnly:
      await record(rig, fixture, url: last, audio: false)
      expected = [.recordStarted, .recordFinished(end: .completed)]
    case .limit:
      await record(rig, fixture, url: last, maxDurationMs: 100, stop: false)
      expected = [flow, .recordStarted, .recordFinished(end: .limitReached)]
    case .revokedTicket:
      rig.begin(fixture, url: first, ticket: ticket)
      ticket.revoke(.inCall)
      rig.open(fixture)
      #expect(await rig.nextRefusal() == .inCall)
      expected = [flow] + expected
      await record(rig, fixture, url: last)
    case .cancelledMidTake:
      rig.begin(fixture, url: first)
      rig.open(fixture)
      guard await rig.next() == flow, case .recordStarted? = await rig.next() else { Issue.record("no flow and started"); return }
      rig.relay.cancel()
      await record(rig, fixture, url: last)
      expected = [flow, .recordStarted, flow, .recordStarted, .recordFinished(end: .completed)]
    }
    let observed = await rig.settle()
    #expect(observed == expected)
    #expect(!rig.relay.isActive)
    if scenario == .revokedTicket || scenario == .cancelledMidTake { #expect(!FileManager.default.fileExists(atPath: first.path)) }
    let facts = try await scanCaptureFacts(at: last)
    guard case let .read(video) = facts.video else { Issue.record("no video stream: \(facts)"); return }
    #expect(video.samples > 0)
    if scenario == .videoOnly { #expect(facts.audio == .absent) }
    else if case let .read(sound) = facts.audio {
      #expect(sound.samples > 0)
      let bufferStep = CMTime(value: CMTimeValue(Fixture.framesPerAudio), timescale: CMTimeScale(Fixture.sampleRate))
      guard let decoded = try await decodedRange(of: last) else { Issue.record("no decoded audio"); return }
      #expect(abs((sound.firstPTS - decoded.start).seconds) <= bufferStep.seconds)
      #expect(abs((sound.lastPTS - decoded.end).seconds) <= bufferStep.seconds)
      if scenario == .limit { #expect((90...110).contains(sound.durationMs)) }
    }
    else { Issue.record("no audio stream: \(facts)") }
  }
}
#endif

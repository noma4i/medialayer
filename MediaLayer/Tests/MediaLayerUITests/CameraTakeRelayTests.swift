#if os(iOS)
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

  private func url() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("camera-relay-\(UUID().uuidString).mp4")
  }

  private func record(_ rig: Rig, _ fixture: Fixture, url: URL, ticket: RecordingTicket? = nil, audio: Bool = true, maxDurationMs: UInt64 = 0, stop: Bool = true) async {
    rig.begin(fixture, url: url, ticket: ticket, audio: audio, maxDurationMs: maxDurationMs)
    #expect(!FileManager.default.fileExists(atPath: url.path))
    rig.open(fixture, audio: audio)
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
    var expected: [CaptureEvent] = [.recordStarted, .recordFinished(end: .completed)]
    switch scenario {
    case .audioAndVideo:
      await record(rig, fixture, url: last, ticket: ticket)
    case .videoOnly:
      await record(rig, fixture, url: last, audio: false)
    case .limit:
      await record(rig, fixture, url: last, maxDurationMs: 100, stop: false)
      expected = [.recordStarted, .recordFinished(end: .limitReached)]
    case .revokedTicket:
      rig.begin(fixture, url: first, ticket: ticket)
      ticket.revoke(.inCall)
      rig.open(fixture)
      #expect(await rig.nextRefusal() == .inCall)
      await record(rig, fixture, url: last)
    case .cancelledMidTake:
      rig.begin(fixture, url: first)
      rig.open(fixture)
      guard case .recordStarted? = await rig.next() else { Issue.record("no started"); return }
      rig.relay.cancel()
      await record(rig, fixture, url: last)
      expected = [.recordStarted, .recordStarted, .recordFinished(end: .completed)]
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
      if scenario == .limit { #expect((90...110).contains(sound.durationMs)) }
    }
    else { Issue.record("no audio stream: \(facts)") }
  }
}
#endif

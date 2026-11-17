#if os(iOS)
import CoreMedia
import Foundation
import Testing
@testable import MediaLayer
@testable import MediaLayerUI

@Suite(.timeLimit(.minutes(1)))
struct CameraTakeRelayTests {
  private typealias Fixture = CameraTakeTests.Fixture

  private final class Rig: @unchecked Sendable {
    let relay: CameraTakeRelay
    var events: AsyncStream<CaptureEvent>.Iterator
    let refusals: AsyncStream<RecordingRefusal>
    let refused: AsyncStream<RecordingRefusal>.Continuation
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("camera-relay-\(UUID().uuidString).mp4")

    init() {
      let (stream, sink) = AsyncStream<CaptureEvent>.makeStream()
      (refusals, refused) = AsyncStream<RecordingRefusal>.makeStream()
      events = stream.makeAsyncIterator()
      relay = CameraTakeRelay(queue: DispatchQueue(label: "camera-relay-test"), emit: { sink.yield($0) }, idle: {})
    }

    func begin(_ fixture: Fixture, ticket: RecordingTicket?, maxDurationMs: UInt64 = 0) {
      var settings = fixture.settings
      settings.maxDurationMs = maxDurationMs
      let refused = refused
      relay.begin(url: url, settings: settings, ticket: ticket) { refused.yield($0) }
    }

    func feed(_ fixture: Fixture) {
      relay.appendAudio(fixture.stale[0])
      for buffer in fixture.audio { relay.appendAudio(buffer) }
      for buffer in fixture.video { relay.appendVideo(buffer) }
    }
  }

  private func ticket() async throws -> (AudioRig, RecordingTicket) {
    let audio = try AudioRig()
    guard case let .success(ticket) = await audio.link.admit(.video) else { throw CancellationError() }
    return (audio, ticket)
  }

  @Test
  func inv_capture_48_a_take_through_the_relay_starts_once_and_finishes_once_with_a_file() async throws {
    let fixture = try Fixture.make()
    let (audio, ticket) = try await ticket()
    defer { ticket.end(); _ = audio }
    var rig = Rig()
    defer { try? FileManager.default.removeItem(at: rig.url) }
    rig.begin(fixture, ticket: ticket)
    #expect(!FileManager.default.fileExists(atPath: rig.url.path))
    rig.feed(fixture)
    guard case .recordStarted? = await rig.events.next() else { Issue.record("no started"); return }
    rig.relay.stop()
    guard case .recordFinished(.completed)? = await rig.events.next() else { Issue.record("no completed"); return }
    #expect(FileManager.default.fileExists(atPath: rig.url.path))
    #expect(!rig.relay.isActive)
  }

  @Test
  func inv_capture_49_a_revoked_ticket_refuses_at_the_begin_and_no_file_or_event_follows() async throws {
    let fixture = try Fixture.make()
    let (audio, ticket) = try await ticket()
    defer { ticket.end(); _ = audio }
    var rig = Rig()
    defer { try? FileManager.default.removeItem(at: rig.url) }
    rig.begin(fixture, ticket: ticket)
    ticket.revoke(.inCall)
    rig.feed(fixture)
    var refusals = rig.refusals.makeAsyncIterator()
    #expect(await refusals.next() == .inCall)
    rig.feed(fixture)
    #expect(!rig.relay.isActive)
    #expect(!FileManager.default.fileExists(atPath: rig.url.path))
  }

  @Test
  func inv_capture_50_a_cancel_ends_the_take_without_an_event_or_a_file() async throws {
    let fixture = try Fixture.make()
    let rig = Rig()
    defer { try? FileManager.default.removeItem(at: rig.url) }
    rig.begin(fixture, ticket: nil)
    rig.relay.cancel()
    rig.feed(fixture)
    #expect(!rig.relay.isActive)
    #expect(!FileManager.default.fileExists(atPath: rig.url.path))
  }

  @Test
  func inv_capture_51_the_limit_of_the_profile_ends_the_take_with_the_limit() async throws {
    let fixture = try Fixture.make()
    var rig = Rig()
    defer { try? FileManager.default.removeItem(at: rig.url) }
    rig.begin(fixture, ticket: nil, maxDurationMs: 100)
    rig.feed(fixture)
    guard case .recordStarted? = await rig.events.next() else { Issue.record("no started"); return }
    guard case .recordFinished(.limitReached)? = await rig.events.next() else { Issue.record("no limit end"); return }
  }
}
#endif

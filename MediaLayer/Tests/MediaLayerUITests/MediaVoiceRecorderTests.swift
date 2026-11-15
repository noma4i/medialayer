import Foundation
import AVFoundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class FakeCapture: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private let refusal: VoiceRefusal?
  private let peaks: [Float]
  private let writes: Bool
  private var file: URL?
  private var readCount = 0
  let readEvents = AsyncStream<Void>.makeStream()
  private(set) var limitMs: UInt64 = 0
  private(set) var limit: (@Sendable () -> Void)?

  init(refusal: VoiceRefusal? = nil, peaks: [Float] = [0.5], writes: Bool = true) {
    self.refusal = refusal
    self.peaks = peaks
    self.writes = writes
  }

  var reads: Int { lock.withLock { readCount } }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    if let refusal { throw VoiceRefused(reason: refusal) }
    lock.withLock {
      self.file = file
      limitMs = maxDurationMs
      limit = onLimit
    }
    TestSignal.shared.fire()
  }

  func peak() -> Float {
    let peak = lock.withLock {
      defer { readCount += 1 }
      return peaks[readCount % peaks.count]
    }
    readEvents.continuation.yield(())
    TestSignal.shared.fire()
    return peak
  }

  private var stopCount = 0
  var stops: Int { lock.withLock { stopCount } }

  func stop() throws {
    lock.withLock { stopCount += 1 }
    TestSignal.shared.fire()
    if writes, let file = lock.withLock({ file }) { try Data([1, 2, 3]).write(to: file) }
  }
}

private final class Ticks: @unchecked Sendable {
  private let lock = NSLock()
  private var value: UInt64 = 1000
  func read() -> UInt64 { lock.withLock { value } }
  func advance(_ ms: UInt64) { lock.withLock { value += ms } }
}

private final class FirstPlayback: @unchecked Sendable {
  private let lock = NSLock()
  private var calls: [String]?

  var log: [String]? { lock.withLock { calls } }

  func record(_ log: [String]) { lock.withLock { if calls == nil { calls = log } }; TestSignal.shared.fire() }
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaVoiceRecorderTests {
  private func file() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
  }

  private func makeRecorder(_ capture: VoiceCapture, ticks: Ticks = Ticks(), audio: MediaAudioSession? = nil, maxMs: UInt64 = 60_000) -> MediaVoiceRecorder {
    MediaVoiceRecorder(capture: capture, maxDurationMs: maxMs, audio: { audio }, clock: { ticks.read() }, levelInterval: .milliseconds(5))
  }

  @Test
  func inv_voice_04_a_recording_gives_an_existing_file_and_a_length_above_zero() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks)
    let target = file()

    try await recorder.start(to: target)
    ticks.advance(2500)
    let recording = try await recorder.stop()

    #expect(recording.url == target)
    #expect(FileManager.default.fileExists(atPath: target.path))
    #expect(recording.durationMs == 2500)
    #expect(!recorder.isRecording)
  }

  @Test
  func inv_voice_05_the_levels_follow_the_peaks_through_the_core_formula_and_stay_in_zero_to_one() async throws {
    let peaks: [Float] = [0, 0.03, 1, 5]
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(peaks: peaks), ticks: ticks)
    var iterator = recorder.levels.makeAsyncIterator()

    try await recorder.start(to: file())
    var seen: [Float] = []
    for _ in peaks { if let level = await iterator.next() { seen.append(level) } }
    ticks.advance(1000)
    _ = try await recorder.stop()

    #expect(seen == peaks.map { voiceLevel(peak: $0) })
    #expect(seen.allSatisfy { (0 ... 1).contains($0) })
  }

  @Test
  func inv_voice_06_the_level_is_not_read_in_rest_or_after_the_recording_ends() async throws {
    let capture = FakeCapture()
    let ticks = Ticks()
    let levelTicks = AsyncStream<Void>.makeStream()
    let recorder = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, audio: { nil }, clock: { ticks.read() }, waitForLevel: {
      var iterator = levelTicks.stream.makeAsyncIterator()
      _ = await iterator.next()
    })
    var reads = capture.readEvents.stream.makeAsyncIterator()
    #expect(recorder.levelPoll.map { _ in true } == nil)
    #expect(capture.reads == 0)

    try await recorder.start(to: file())
    let poll = try #require(recorder.levelPoll)
    for _ in 0..<2 {
      levelTicks.continuation.yield(())
      _ = await reads.next()
    }
    ticks.advance(1000)
    _ = try await recorder.stop()
    await poll.value
    let after = capture.reads
    levelTicks.continuation.yield(())

    #expect(recorder.levelPoll.map { _ in true } == nil)
    #expect(capture.reads == after)
  }

  @Test
  func inv_voice_07_a_refused_permission_is_a_media_error_with_the_text_of_the_core() async throws {
    for reason in [VoiceRefusal.permissionDenied, .microphoneBusy] {
      let recorder = makeRecorder(FakeCapture(refusal: reason))
      do {
        try await recorder.start(to: file())
        Issue.record("the start was not refused")
      } catch let error as MediaError {
        guard case let .Voice(detail) = error else { Issue.record("wrong error \(error)"); continue }
        #expect(detail == voiceRefusalText(reason: reason))
      }
      #expect(!recorder.isRecording)
    }
  }

  @Test
  func inv_voice_08_the_recording_takes_the_audio_focus_and_gives_it_back_at_the_end() async throws {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, audio: session)

    try await recorder.start(to: file())
    #expect(session.isRecording)
    ticks.advance(1000)
    _ = try await recorder.stop()

    #expect(!session.isRecording)
  }

  @Test(arguments: [0, 4])
  func playbackAfterRecordingStartsWithAnActivePlaybackSession(busyDeactivations: Int) async throws {
    let port = FakeAudioSessionPort(busyDeactivations: busyDeactivations)
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, audio: session)
    let target = file()
    defer { try? FileManager.default.removeItem(at: target) }
    try await recorder.start(to: target)
    ticks.advance(1000)
    _ = try await recorder.stop()

    let factory = AVQueuePlayerSlotFactory(startMuted: false, audio: session)
    let slot = await factory.makeSlot(7)
    let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!
    await slot.load(key: "voice", route: PlaybackRoute(url: clip.absoluteString, headers: [:], generation: nil, hls: false, looping: true))
    let first = FirstPlayback()
    let observation = await MainActor.run {
      slot.player.observe(\.timeControlStatus, options: [.new]) { player, _ in
        if player.timeControlStatus != .paused { first.record(port.log) }
      }
    }
    await slot.play()
    await eventually("player started") { first.log != nil }
    let calls = try #require(first.log)
    #expect(calls.last(where: { $0.hasPrefix("category ") }) == FakeAudioSessionPort.category(.playback(.mix)))
    #expect(calls.last(where: { $0.hasPrefix("active ") && !$0.hasSuffix("BUSY") }) == "active true")
    observation.invalidate()
    await slot.unload()
  }

  @Test
  func inv_voice_09_the_limit_ends_the_recording_by_itself_and_stop_gives_its_file() async throws {
    let ticks = Ticks()
    let capture = FakeCapture()
    let recorder = makeRecorder(capture, ticks: ticks, maxMs: 30_000)

    try await recorder.start(to: file())
    #expect(capture.limitMs == 30_000)
    ticks.advance(30_000)
    capture.limit?()
    let recording = try await recorder.stop()

    #expect(recording.durationMs == 30_000)
    #expect(FileManager.default.fileExists(atPath: recording.url.path))
    #expect(capture.stops == 1, "the recording ended once")
  }

  @Test
  func inv_voice_10_a_recording_without_sound_is_a_media_error_and_leaves_no_file() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(writes: false), ticks: ticks)
    let target = file()

    try await recorder.start(to: target)
    ticks.advance(1000)
    do {
      _ = try await recorder.stop()
      Issue.record("an empty recording was accepted")
    } catch let error as MediaError {
      guard case let .Voice(detail) = error else { Issue.record("wrong error \(error)"); return }
      #expect(detail == voiceRefusalText(reason: .empty))
    }
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_voice_11_the_system_capture_records_aac_in_one_channel() {
    #expect(SystemVoiceCapture.settings[AVFormatIDKey] as? Int == Int(kAudioFormatMPEG4AAC))
    #expect(SystemVoiceCapture.settings[AVNumberOfChannelsKey] as? Int == 1)
  }

  @Test
  func inv_voice_12_the_system_capture_records_at_44100_hz_and_128000_bit_per_second() {
    #expect(SystemVoiceCapture.settings[AVSampleRateKey] as? Int == 44_100)
    #expect(SystemVoiceCapture.settings[AVNumberOfChannelsKey] as? Int == 1)
    #expect(SystemVoiceCapture.settings[AVEncoderBitRateKey] as? Int == 128_000)
  }
}

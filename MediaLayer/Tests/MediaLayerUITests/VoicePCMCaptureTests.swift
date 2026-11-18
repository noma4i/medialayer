#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

/// The native PCM boundary of a voice take, the only fake of these tests. The audio core, the ticket, the owner of the input, `SystemVoiceCapture`, the m4a writer and `MediaVoiceRecorder` are the real ones.
private final class FakePCMSource: VoicePCMSource, @unchecked Sendable {
  private let lock = NSLock()
  private var started: [UInt64] = []
  private var stopped: [UInt64] = []
  private var sink: VoicePCMSink?
  private var emitted: Int64 = 0
  private var onStart: (@Sendable (VoicePCMSink, UInt64) -> Void)?

  var starts: [UInt64] { lock.withLock { started } }
  var stops: [UInt64] { lock.withLock { stopped } }
  var generation: UInt64 { lock.withLock { started.last ?? 0 } }

  /// Runs inside `start`, on the thread of the owner: a native callback that comes before anything else can register.
  func whenStarted(_ action: (@Sendable (VoicePCMSink, UInt64) -> Void)?) { lock.withLock { onStart = action } }

  func start(generation: UInt64, sink: VoicePCMSink) {
    let action = lock.withLock { () -> (@Sendable (VoicePCMSink, UInt64) -> Void)? in
      started.append(generation)
      self.sink = sink
      return onStart
    }
    action?(sink, generation)
    TestSignal.shared.fire()
  }

  func stop(generation: UInt64) {
    lock.withLock { stopped.append(generation) }
    TestSignal.shared.fire()
  }

  func emit(_ count: Int = 1, generation: UInt64? = nil) throws {
    for _ in 0..<count {
      let (sink, first, current) = lock.withLock { () -> (VoicePCMSink?, Int64, UInt64) in
        defer { emitted += pcmFrames }
        return (self.sink, emitted, started.last ?? 0)
      }
      sink?.pcm(generation: generation ?? current, try pcmBuffer(firstFrame: first))
    }
  }

  func drop() {
    let (sink, generation) = lock.withLock { (self.sink, started.last ?? 0) }
    sink?.dropped(generation: generation)
  }

  func lose(_ error: Error? = nil) {
    let (sink, generation) = lock.withLock { (self.sink, started.last ?? 0) }
    sink?.lost(generation: generation, error: error)
  }
}

private let pcmRate: Double = 44_100
private let pcmFrames: Int64 = 4_410

private func pcmBuffer(firstFrame: Int64) throws -> CMSampleBuffer {
  var description = AudioStreamBasicDescription(
    mSampleRate: pcmRate, mFormatID: kAudioFormatLinearPCM, mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
    mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0
  )
  var format: CMAudioFormatDescription?
  try check(CMAudioFormatDescriptionCreate(allocator: nil, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format))
  let length = Int(pcmFrames) * 2
  var block: CMBlockBuffer?
  try check(CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: length, blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: length, flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block))
  try check(CMBlockBufferFillDataBytes(with: 0, blockBuffer: block!, offsetIntoDestination: 0, dataLength: length))
  var buffer: CMSampleBuffer?
  let pts = CMTime(value: 1_000 * Int64(pcmRate) + firstFrame, timescale: CMTimeScale(pcmRate))
  try check(CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: nil, dataBuffer: block!, formatDescription: format!, sampleCount: CMItemCount(pcmFrames), presentationTimeStamp: pts, packetDescriptions: nil, sampleBufferOut: &buffer))
  return buffer!
}

private func check(_ status: OSStatus) throws {
  guard status == noErr else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
}

private final class BrokenLog: @unchecked Sendable {
  private let lock = NSLock()
  private var list: [String] = []
  func add(_ value: String) { lock.withLock { list.append(value) } }
  var values: [String] { lock.withLock { list } }
}

/// A writer boundary that stays ready for a number of buffers and then is not: the bounded resource of the take.
private final class BoundedWriter: VoiceFileWriter, @unchecked Sendable {
  private let url: URL
  private let capacity: Int
  private let lock = NSLock()
  private var appended = 0

  init(url: URL, capacity: Int) { self.url = url; self.capacity = capacity }

  var isReady: Bool { lock.withLock { appended < capacity } }
  func open(first: CMSampleBuffer, settings: [String: Any]) throws { try Data([1]).write(to: url) }
  func append(_ buffer: CMSampleBuffer) throws { lock.withLock { appended += 1 } }
  func finish() async throws {}
  func cancel() {}
}

/// A writer whose close does not end until the test lets it: the native completion of a file.
private final class HeldWriter: VoiceFileWriter, @unchecked Sendable {
  private let url: URL
  private let lock = NSLock()
  private var release: CheckedContinuation<Void, Never>?
  private var closing = false

  init(url: URL) { self.url = url }

  var isReady: Bool { true }
  var isClosing: Bool { lock.withLock { closing } }
  func open(first: CMSampleBuffer, settings: [String: Any]) throws { try Data([1]).write(to: url) }
  func append(_ buffer: CMSampleBuffer) throws {}
  func cancel() {}

  func finish() async throws {
    await withCheckedContinuation { continuation in
      lock.withLock { closing = true; release = continuation }
      TestSignal.shared.fire()
    }
  }

  func complete() { lock.withLock { () -> CheckedContinuation<Void, Never>? in defer { release = nil }; return release }?.resume() }
}

/// The real capture with one native event fired after its start returned and before the recorder commits: the window of an early end.
private final class GatedCapture: VoiceCapture, @unchecked Sendable {
  private let inner: SystemVoiceCapture
  private let afterStart: @Sendable () -> Void

  init(_ inner: SystemVoiceCapture, afterStart: @escaping @Sendable () -> Void) { self.inner = inner; self.afterStart = afterStart }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    try await inner.start(file: file, maxDurationMs: maxDurationMs, onLimit: onLimit, onFailure: onFailure)
    afterStart()
  }

  func start(file: URL, maxDurationMs: UInt64, ticket: RecordingTicket, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    try await inner.start(file: file, maxDurationMs: maxDurationMs, ticket: ticket, onLimit: onLimit, onFailure: onFailure)
    afterStart()
  }

  func peak() -> Float { inner.peak() }
  func stop() throws { try inner.stop() }
  func finish() async throws -> UInt64? { try await inner.finish() }
}

private final class Returned: @unchecked Sendable {
  private let lock = NSLock()
  private var flag = false
  private var failure: Error?
  var done: Bool { lock.withLock { flag } }
  var error: Error? { lock.withLock { failure } }
  func finish(_ error: Error? = nil) { lock.withLock { flag = true; failure = error }; TestSignal.shared.fire() }
}

private final class Clock: @unchecked Sendable {
  private let lock = NSLock()
  private var now: UInt64 = 1_000
  private let step: UInt64

  init(step: UInt64 = 0) { self.step = step }

  func read() -> UInt64 { lock.withLock { defer { now += step }; return now } }
  func advance(_ ms: UInt64) { lock.withLock { now += ms } }
}

private struct Voice {
  let rig: AudioRig
  let source = FakePCMSource()
  let clock: Clock
  let recorder: MediaVoiceRecorder

  init(maxMs: UInt64 = 60_000, clockStep: UInt64 = 0, writer: @escaping @Sendable (URL) -> VoiceFileWriter = { AssetVoiceWriter(url: $0) }, afterStart: (@Sendable (FakePCMSource) -> Void)? = nil) throws {
    let rig = try AudioRig()
    let source = source
    let clock = Clock(step: clockStep)
    self.clock = clock
    self.rig = rig
    let system = SystemVoiceCapture(source: source, link: { rig.link }, writer: writer)
    recorder = MediaVoiceRecorder(
      capture: afterStart.map { gate in GatedCapture(system, afterStart: { gate(source) }) } ?? system, maxDurationMs: maxMs,
      link: { rig.link }, clock: { clock.read() }, levelInterval: .milliseconds(5)
    )
  }

  func begin(_ file: URL) -> (Task<Void, Never>, Returned) {
    let returned = Returned()
    let recorder = recorder
    return (Task { do { try await recorder.start(to: file); returned.finish() } catch { returned.finish(error) } }, returned)
  }

  func running(_ file: URL) async throws {
    let (task, returned) = begin(file)
    await eventually("the core connects the source of the take") { source.starts.count > 0 && source.generation > 0 }
    try source.emit()
    await task.value
    #expect(returned.error == nil)
  }
}

private func voiceRefusal(_ error: Error?) -> VoiceRefusal? {
  guard case let .Voice(_, reason)? = error as? MediaError else { return nil }
  return reason
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct VoicePCMCaptureTests {
    @Test
    func aVoiceFileAndTheRecordingStateComeFromTheFirstLivePCMAndStopGivesAnM4a() async throws {
      let voice = try Voice()
      let file = temporaryFile("m4a")
      let (task, returned) = voice.begin(file)

      await eventually("the core connects the one source of the take") { voice.source.starts == [1] }
      #expect(!FileManager.default.fileExists(atPath: file.path), "no file before the first live PCM")
      #expect(!returned.done, "start waits for the first live PCM")
      #expect(voice.recorder.state.phase == .idle)
      await eventually("the public availability says no sound before the PCM") { voice.rig.availability.last == .noSound }

      try voice.source.emit()
      await task.value

      #expect(returned.error == nil)
      #expect(FileManager.default.fileExists(atPath: file.path), "the writer started with the first PCM")
      #expect(voice.recorder.state.phase == .recording)
      await eventually("the public availability says available after the PCM") { voice.rig.availability.last == .available }
      #expect(voice.source.starts == [1], "one native source")
      try voice.source.emit(9)
      voice.clock.advance(1_000)
      let recording = try await voice.recorder.stop()

      #expect(recording.url == file && recording.durationMs == 1_000)
      let asset = AVURLAsset(url: file)
      let tracks = try await asset.loadTracks(withMediaType: .audio)
      #expect(tracks.count == 1)
      let formats = try await tracks[0].load(.formatDescriptions)
      let encoder = voiceEncoderSettings()
      let basic = formats.first.flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }
      #expect(basic?.mFormatID == kAudioFormatMPEG4AAC)
      #expect(basic?.mChannelsPerFrame == encoder.channels)
      #expect(basic?.mSampleRate == Double(encoder.sampleRateHz))
      #expect(try await asset.load(.duration).seconds > 0)
      await eventually("the source stopped with the take") { voice.source.stops == [1] }
    }

    @Test
    func aCancelledStartEndsOnceWithoutAFileAndFreesTheSource() async throws {
      let voice = try Voice()
      let file = temporaryFile("m4a")
      let (task, returned) = voice.begin(file)
      await eventually("connected") { voice.source.starts == [1] }

      task.cancel()
      await task.value

      #expect(returned.error is CancellationError)
      #expect(!FileManager.default.fileExists(atPath: file.path))
      #expect(voice.recorder.state.phase == .idle)
      await eventually("the source is stopped and the microphone is free") { voice.source.stops == [1] }
      try voice.source.emit()
      #expect(!FileManager.default.fileExists(atPath: file.path), "a PCM of an ended wait creates nothing")
    }

    @Test
    func aSourceWithoutSoundWhileWaitingIsOneMicrophoneSilentWithoutAFile() async throws {
      let voice = try Voice()
      let file = temporaryFile("m4a")
      let (task, returned) = voice.begin(file)
      await eventually("connected") { voice.source.starts == [1] }

      voice.source.lose()
      await task.value

      #expect(voiceRefusal(returned.error) == .microphoneSilent)
      #expect(!FileManager.default.fileExists(atPath: file.path))
      #expect(voice.recorder.state == VoiceState(take: 0, phase: .idle, outcome: .refused(reason: .microphoneSilent), recordedMs: 0))
    }

    @Test
    func aRefusedPermissionOfTheSourceIsTypedAndANativeErrorStaysAFailure() async throws {
      for (error, expected) in [
        (NSError(domain: AVFoundationErrorDomain, code: AVError.applicationIsNotAuthorizedToUseDevice.rawValue), VoiceRefusal.permissionDenied),
        (NSError(domain: AVFoundationErrorDomain, code: AVError.deviceNotConnected.rawValue), .microphoneBusy)
      ] {
        let voice = try Voice()
        let (task, returned) = voice.begin(temporaryFile("m4a"))
        await eventually("connected") { voice.source.starts == [1] }
        voice.source.lose(error)
        await task.value
        #expect(voiceRefusal(returned.error) == expected)
      }
      let voice = try Voice()
      let (task, returned) = voice.begin(temporaryFile("m4a"))
      await eventually("connected") { voice.source.starts == [1] }
      voice.source.lose(NSError(domain: "native", code: 7))
      await task.value
      #expect(voiceRefusal(returned.error) == nil && returned.error != nil, "a native error is not a refusal")
    }

    @Test
    func inv_voice_26_aLossOfSoundWhileRecordingIsOneMicrophoneSilentWithoutAFileAndTheOwnerClosesAtOnce() async throws {
      let voice = try Voice()
      let file = temporaryFile("m4a")
      let states = Collected(voice.recorder.states)
      try await voice.running(file)
      await eventually("recording") { states.last?.phase == .recording }

      voice.source.lose()

      await eventually("one outcome") { states.last?.outcome != nil }
      #expect(states.last == VoiceState(take: 1, phase: .idle, outcome: .refused(reason: .microphoneSilent), recordedMs: 0))
      #expect(!FileManager.default.fileExists(atPath: file.path))
      await eventually("the owner is closed without a restore") { voice.source.stops == [1] }
      let count = states.values.count

      try voice.source.emit(3)

      #expect(voice.source.starts == [1], "no restore of a source without a take")
      #expect(states.values.count == count, "no take resumed")
      #expect(!FileManager.default.fileExists(atPath: file.path))
      await eventually("the public availability is available after the owner closed") { voice.rig.availability.last == .available }
      await #expect(throws: MediaError.self) { _ = try await voice.recorder.stop() }
    }

    @Test
    func aLossBetweenTheFirstPCMAndTheCommitOfTheStartIsTheResultOfTheStart() async throws {
      let voice = try Voice(afterStart: { $0.lose() })
      let file = temporaryFile("m4a")
      let states = Collected(voice.recorder.states)
      let (task, returned) = voice.begin(file)
      await eventually("connected") { voice.source.starts == [1] }

      try voice.source.emit()
      await task.value

      #expect(voiceRefusal(returned.error) == .microphoneSilent)
      #expect(states.last == VoiceState(take: 0, phase: .idle, outcome: .refused(reason: .microphoneSilent), recordedMs: 0))
      #expect(states.values.filter { $0.outcome != nil }.count == 1 && states.values.allSatisfy { $0.phase == .idle })
      #expect(!FileManager.default.fileExists(atPath: file.path))
      #expect(!voice.recorder.isRecording)
    }

    @Test
    func aLimitReachedByTheFirstPCMEndsTheTakeAtOnceAfterTheCommitOfTheStart() async throws {
      let voice = try Voice(maxMs: 50, clockStep: 1_000)
      let file = temporaryFile("m4a")
      let states = Collected(voice.recorder.states)
      let (task, returned) = voice.begin(file)
      await eventually("connected") { voice.source.starts == [1] }

      try voice.source.emit()
      await task.value

      #expect(returned.error == nil)
      await eventually("the take ended by its limit") { states.last?.phase == .idle && states.last?.outcome != nil }
      guard case let .recorded(path: path, durationMs: _)? = states.last?.outcome else {
        Issue.record("the outcome is \(String(describing: states.last?.outcome))")
        return
      }
      #expect(path == file.path && FileManager.default.fileExists(atPath: file.path))
      #expect(states.values.filter { $0.outcome != nil }.count == 1)
      #expect(try await voice.recorder.stop().url == file)
    }

    @Test
    func aNativeCallbackThatComesWhileTheOwnerOpensIsTheLossOfItsTake() async throws {
      let voice = try Voice()
      voice.source.whenStarted { sink, generation in
        DispatchQueue.global().async { sink.lost(generation: generation, error: nil) }
      }
      let states = Collected(voice.recorder.states)
      let (task, returned) = voice.begin(temporaryFile("m4a"))
      await task.value
      voice.source.whenStarted(nil)

      #expect(voiceRefusal(returned.error) == .microphoneSilent)
      #expect(states.last == VoiceState(take: 0, phase: .idle, outcome: .refused(reason: .microphoneSilent), recordedMs: 0))

      let before = voice.source.starts.count
      let file = temporaryFile("m4a")
      let (next, again) = voice.begin(file)
      await eventually("the next take opens its own input") { voice.source.starts.count == before + 1 }
      try voice.source.emit()
      await next.value
      #expect(again.error == nil && voice.recorder.state.phase == .recording)
      voice.clock.advance(1_000)
      _ = try await voice.recorder.stop()
    }

    @Test
    func aCancelWhileTheFileClosesRemovesItAfterTheNativeCompletionAndTheResultIsTheCancel() async throws {
      let file = temporaryFile("m4a")
      let writer = HeldWriter(url: file)
      let take = VoiceTake(url: file, writer: writer, settings: [:], maxDurationMs: 60_000, onStarted: {}, onLimit: {}, onBroken: { _ in })
      take.pcm(generation: 1, try pcmBuffer(firstFrame: 0))
      #expect(FileManager.default.fileExists(atPath: file.path))
      let closing = Task { try await take.finish() }
      await eventually("the file is closing") { writer.isClosing }

      take.cancel()
      #expect(FileManager.default.fileExists(atPath: file.path), "the file is removed by the native completion, not before")
      writer.complete()

      await #expect(throws: CancellationError.self) { try await closing.value }
      #expect(!FileManager.default.fileExists(atPath: file.path))
    }

    @Test
    func inv_voice_33_finish_gives_the_length_of_the_written_audio_in_microseconds() async throws {
      let file = temporaryFile("m4a")
      let writer = HeldWriter(url: file)
      let take = VoiceTake(url: file, writer: writer, settings: [:], maxDurationMs: 60_000, onStarted: {}, onLimit: {}, onBroken: { _ in })
      for index in 0..<3 { take.pcm(generation: 1, try pcmBuffer(firstFrame: Int64(index) * pcmFrames)) }
      let closing = Task { try await take.finish() }
      await eventually("the file is closing") { writer.isClosing }
      writer.complete()
      #expect(try await closing.value == 300_000)
      #expect(try await VoiceTake(url: file, writer: writer, settings: [:], maxDurationMs: 60_000, onStarted: {}, onLimit: {}, onBroken: { _ in }).finish() == 0, "a take that never wrote has no length")
    }

    /// O-21: audio that reaches the real writer far faster than real time is written whole: a not-ready input only makes the buffers wait.
    @Test
    func inv_voice_31_a_burst_of_pcm_is_written_whole_by_the_real_writer() async throws {
      let file = temporaryFile("m4a")
      defer { try? FileManager.default.removeItem(at: file) }
      let settings: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: pcmRate, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 64_000]
      let broken = BrokenLog()
      let take = VoiceTake(url: file, writer: AssetVoiceWriter(url: file), settings: settings, maxDurationMs: 600_000, onStarted: {}, onLimit: {}, onBroken: { broken.add("\($0)") })
      let buffers = 600
      for index in 0..<buffers { take.pcm(generation: 1, try pcmBuffer(firstFrame: Int64(index) * pcmFrames)) }
      let length = try await take.finish()
      #expect(broken.values.isEmpty, "\(broken.values)")
      #expect(length == UInt64(buffers) * 100_000)
      #expect(FileManager.default.fileExists(atPath: file.path))
    }

    @Test
    func inv_voice_23_the_length_of_the_result_is_the_written_audio_not_the_clock() async throws {
      let voice = try Voice()
      let file = temporaryFile("m4a")
      try await voice.running(file)
      try voice.source.emit(4)
      voice.clock.advance(3_000)
      let recording = try await voice.recorder.stop()
      #expect(recording.durationMs == 500, "five buffers of 100 ms are written, whatever the clock says")
    }

    @Test
    func inv_voice_31_a_buffer_dropped_by_the_capture_is_one_typed_failure_without_a_file_and_the_owner_is_closed() async throws {
      let voice = try Voice(writer: { BoundedWriter(url: $0, capacity: 3) })
      let file = temporaryFile("m4a")
      let failures = Collected(Media.failures)
      let states = Collected(voice.recorder.states)
      try await voice.running(file)
      try voice.source.emit(2)
      #expect(FileManager.default.fileExists(atPath: file.path), "the bounded writer has the file while it takes buffers")

      voice.source.drop()

      await eventually("one typed failure") { if case .failed? = states.last?.outcome { true } else { false } }
      #expect(!FileManager.default.fileExists(atPath: file.path))
      #expect(states.values.filter { $0.outcome != nil }.count == 1)
      #expect(failures.values.filter { $0.source == .recorder }.count == 1, "\(failures.values)")
      await eventually("the owner is closed with the failed take") { voice.source.stops == [1] }
      #expect(voice.source.starts == [1], "a failed writer does not wait for a restore")
    }

    @Test
    func inv_voice_26_aSourceThatWasLostIsFreedAtOnceWhetherOrNotTheLayerOrTheRecorderGoes() async throws {
      for close in [true, false] {
        let rig = try AudioRig()
        let source = FakePCMSource()
        var recorder: MediaVoiceRecorder? = MediaVoiceRecorder(capture: SystemVoiceCapture(source: source, link: { rig.link }), maxDurationMs: 60_000, link: { rig.link }, levelInterval: .milliseconds(5))
        let task = Task { [recorder] in try? await recorder!.start(to: temporaryFile("m4a")) }
        await eventually("connected") { source.starts == [1] }
        try source.emit()
        await task.value
        source.lose()

        await eventually("the owner is freed at once (layer close \(close))") { source.stops == [1] }
        #expect(source.starts == [1], "no restore of a source without a take")
        if close { rig.link.close() } else { recorder = nil }
        withExtendedLifetime(recorder) {}
      }
    }

    @Test
    func aCallWhileWaitingOrRecordingEndsTheTakeWithInCallAndWithoutAFile() async throws {
      for running in [false, true] {
        let voice = try Voice()
        let file = temporaryFile("m4a")
        let states = Collected(voice.recorder.states)
        let returned: Returned
        let task: Task<Void, Never>
        if running {
          try await voice.running(file)
          (task, returned) = (Task {}, Returned())
        } else {
          (task, returned) = voice.begin(file)
          await eventually("connected") { voice.source.starts == [1] }
        }

        let call = try await voice.rig.link.call(.unmanaged)
        await task.value

        if !running { #expect(voiceRefusal(returned.error) == .inCall, "waiting") }
        await eventually("the take ended with inCall (running \(running))") { states.last?.outcome == .refused(reason: .inCall) }
        #expect(!FileManager.default.fileExists(atPath: file.path))
        await eventually("the call stopped the source") { voice.source.stops.contains(1) }
        call.end()
      }
    }

    @Test
    func aTakeAfterAStoppedTakeStartsItsOwnSourceAndFile() async throws {
      let voice = try Voice()
      let first = temporaryFile("m4a")
      try await voice.running(first)
      voice.clock.advance(1_000)
      _ = try await voice.recorder.stop()
      await eventually("the first source is stopped") { voice.source.stops == [1] }

      let second = temporaryFile("m4a")
      let (task, returned) = voice.begin(second)
      await eventually("the second take connects its own source") { voice.source.starts.count == 2 }
      #expect(!FileManager.default.fileExists(atPath: second.path), "no file before the PCM of the second take")
      try voice.source.emit()
      await task.value
      #expect(returned.error == nil && FileManager.default.fileExists(atPath: second.path))
      voice.clock.advance(1_000)
      let recording = try await voice.recorder.stop()
      #expect(recording.url == second && recording.durationMs == 100)
      #expect(FileManager.default.fileExists(atPath: first.path))
    }
  }
}
#endif

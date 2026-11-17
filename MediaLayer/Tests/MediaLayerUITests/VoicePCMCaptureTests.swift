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
  func finish() async throws { try await inner.finish() }
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
  }
}
#endif

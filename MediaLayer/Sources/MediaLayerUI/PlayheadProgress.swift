import Foundation
import MediaLayer
import SwiftUI

/// The report of a take of a camera or a voice recorder for the one playhead rule of the core: the position is the recorded time, the duration is the limit of the take.
func takeReport(recordedMs: UInt64, recording: Bool, take: UInt64?, limitMs: UInt64, nowMs: UInt64, previousTake: UInt64?) -> PlayheadReport {
  PlayheadReport(
    positionMs: recordedMs,
    durationMs: limitMs,
    atMs: nowMs,
    rate: 1,
    playing: recording,
    jump: take != previousTake,
    settling: false
  )
}

/// The position of the video a source plays, drawn between the reports of the player. The view only reads: it mounts and plays nothing.
public struct MediaVideoProgress<Content: View>: View {
  @ResolvedMediaContext private var context
  @State private var playhead = AudioPlayhead()
  private let item: MediaItem
  private let content: (Double) -> Content

  public init(_ source: some MediaSource, @ViewBuilder content: @escaping (Double) -> Content) {
    self.item = source.item
    self.content = content
  }

  public var body: some View {
    TimelineView(.animation(paused: playhead.anchor.perMs == 0)) { context in
      content(playhead.progress(at: context.date))
    }
    .task(id: item.url) { await follow() }
  }

  private func follow() async {
    playhead = AudioPlayhead()
    guard let slots = context?.slots else { return }
    let key = await SourceKey.of(item)
    guard !Task.isCancelled else { return }
    for await report in slots.progress.reports(for: key) {
      playhead = AudioPlayhead(anchor: playheadNext(shown: playhead.anchor, report: report))
    }
  }
}

/// The progress of the running take of a camera against the limit of its profile, or of a voice recorder against its limit, drawn between the published states.
public struct MediaTakeProgress<Content: View>: View {
  private enum Source {
    case camera(MediaCamera)
    case voice(MediaVoiceRecorder)

    var id: ObjectIdentifier {
      switch self {
      case .camera(let camera): ObjectIdentifier(camera)
      case .voice(let recorder): ObjectIdentifier(recorder)
      }
    }
  }

  @State private var playhead = AudioPlayhead()
  private let source: Source
  private let content: (Double) -> Content

  public init(_ camera: MediaCamera, @ViewBuilder content: @escaping (Double) -> Content) {
    self.source = .camera(camera)
    self.content = content
  }

  /// The same ring for a voice recorder: the recorded time against its limit. A recorder without a limit (0) gives 0.
  public init(_ recorder: MediaVoiceRecorder, @ViewBuilder content: @escaping (Double) -> Content) {
    self.source = .voice(recorder)
    self.content = content
  }

  public var body: some View {
    TimelineView(.animation(paused: playhead.anchor.perMs == 0)) { context in
      content(playhead.progress(at: context.date))
    }
    .task(id: source.id) { await follow() }
  }

  private func follow() async {
    playhead = AudioPlayhead()
    var previous: UInt64?
    switch source {
    case .camera(let camera):
      for await state in camera.states {
        previous = apply(recordedMs: state.recordedMs, recording: state.phase == .recording, take: state.take, limitMs: camera.limitMs, previous: previous)
      }
    case .voice(let recorder):
      for await state in recorder.states {
        previous = apply(recordedMs: state.recordedMs, recording: state.phase == .recording, take: state.take, limitMs: recorder.maxDurationMs, previous: previous)
      }
    }
  }

  private func apply(recordedMs: UInt64, recording: Bool, take: UInt64?, limitMs: UInt64, previous: UInt64?) -> UInt64? {
    let report = takeReport(recordedMs: recordedMs, recording: recording, take: take, limitMs: limitMs, nowMs: AudioPlayhead.milliseconds(Date()), previousTake: previous)
    playhead = AudioPlayhead(anchor: playheadNext(shown: playhead.anchor, report: report))
    return take
  }
}

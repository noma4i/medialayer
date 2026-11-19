import Foundation
import MediaLayer
import SwiftUI

/// The report of a camera take for the one playhead rule of the core: the position is the recorded time, the duration is the limit of the profile.
func takeReport(state: CameraState, limitMs: UInt64, nowMs: UInt64, previousTake: UInt64?) -> PlayheadReport {
  PlayheadReport(
    positionMs: state.recordedMs,
    durationMs: limitMs,
    atMs: nowMs,
    rate: 1,
    playing: state.phase == .recording,
    jump: state.take != previousTake,
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

/// The progress of the running take of a camera against the limit of its profile, drawn between the published states.
public struct MediaTakeProgress<Content: View>: View {
  @State private var playhead = AudioPlayhead()
  private let camera: MediaCamera
  private let content: (Double) -> Content

  public init(_ camera: MediaCamera, @ViewBuilder content: @escaping (Double) -> Content) {
    self.camera = camera
    self.content = content
  }

  public var body: some View {
    TimelineView(.animation(paused: playhead.anchor.perMs == 0)) { context in
      content(playhead.progress(at: context.date))
    }
    .task(id: ObjectIdentifier(camera)) { await follow() }
  }

  private func follow() async {
    playhead = AudioPlayhead()
    var previous: UInt64?
    for await state in camera.states {
      let report = takeReport(state: state, limitMs: camera.limitMs, nowMs: AudioPlayhead.milliseconds(Date()), previousTake: previous)
      previous = state.take
      playhead = AudioPlayhead(anchor: playheadNext(shown: playhead.anchor, report: report))
    }
  }
}

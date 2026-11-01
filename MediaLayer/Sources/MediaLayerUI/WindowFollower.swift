import MediaLayer
import Observation
import SwiftUI

enum WindowLayout<Row: Sendable>: Sendable {
  case video(source: @Sendable (Row) -> MediaItem?)
  case image(name: String, rowsPerPage: Int, reversed: Bool, sources: @Sendable (Row) -> [MediaItem])
}

private struct WindowUpdate<Row: Sendable>: Sendable {
  var rows: [Row]?
  var active: Int?
  var visible: ClosedRange<Int>?
  var isActive: Bool?
  var closes = false

  static func merged(_ held: WindowUpdate?, _ next: WindowUpdate) -> WindowUpdate {
    WindowUpdate(
      rows: next.rows ?? held?.rows, active: next.active ?? held?.active, visible: next.visible ?? held?.visible,
      isActive: next.isActive ?? held?.isActive, closes: (held?.closes ?? false) || next.closes
    )
  }
}

/// Holds the latest layout: the mapper that a modifier body passed last is the one the next send uses.
final class WindowLayoutBox<Row: Sendable>: @unchecked Sendable {
  private let lock = NSLock()
  private var layout: WindowLayout<Row>

  init(_ layout: WindowLayout<Row>) { self.layout = layout }

  func use(_ next: WindowLayout<Row>) { lock.withLock { layout = next } }

  var current: WindowLayout<Row> { lock.withLock { layout } }
}

private final class OpenedWindow: @unchecked Sendable {
  var window: (any WindowHandle)?
  var active = 0
  var isClosed = false
}

private final class ReadyRelay: WindowReadyListener, @unchecked Sendable {
  private let lock = NSLock()
  private var latest = false
  private let deliver: @Sendable @MainActor (Bool) -> Void

  init(deliver: @escaping @Sendable @MainActor (Bool) -> Void) { self.deliver = deliver }

  func changed(ready: Bool) {
    lock.withLock { latest = ready }
    Task { @MainActor in deliver(lock.withLock { latest }) }
  }
}

private final class ChangeLatch: @unchecked Sendable {
  private let lock = NSLock()
  private var isRaised = false

  func raise() -> Bool { lock.withLock { defer { isRaised = true }; return !isRaised } }

  func clear() { lock.withLock { isRaised = false } }
}

/// Follows one window of the app list. It holds one running preparation and one latest snapshot: array mapping and the call into the core run off the main thread.
@MainActor
final class WindowFollower<Row: Sendable> {
  private var generation = 0
  private var drain: LatestDrain<WindowUpdate<Row>>?
  private var read: (@MainActor () -> [Row])?
  private var box: WindowLayoutBox<Row>?
  private var latch = ChangeLatch()
  private var report: (@MainActor (Bool) -> Void)?
  private var isReady = false

  func mount(
    places: MediaPlaces, layout: WindowLayout<Row>, data: @escaping @MainActor () -> [Row], active: Int, enabled: Bool,
    tracksRows: Bool = false, failed: @escaping @Sendable (MediaError) -> Void
  ) {
    close()
    generation += 1
    latch = ChangeLatch()
    read = data
    let box = WindowLayoutBox(layout)
    self.box = box
    let opened = OpenedWindow()
    let expected = generation
    let relay = report.map { _ in ReadyRelay { [weak self] ready in self?.reported(ready, expected) } }
    drain = LatestDrain(merge: { WindowUpdate.merged($0, $1) }) { update in
      WindowFollower.apply(update, places: places, layout: box.current, opened: opened, relay: relay, failed: failed)
    }
    drain?.submit(WindowUpdate(active: tracksRows ? nil : active, isActive: enabled))
    observe(generation)
  }

  func use(ready: (@MainActor (Bool) -> Void)?) { report = ready }

  func show(_ rows: ClosedRange<Int>) { drain?.submit(WindowUpdate(visible: rows)) }

  private func reported(_ ready: Bool, _ expected: Int) {
    guard expected == generation, ready != isReady else { return }
    isReady = ready
    report?(ready)
  }

  /// A mapper is a pure function of a row. The latest one serves the next send; rows already sent are not mapped again.
  func use(layout: WindowLayout<Row>) { box?.use(layout) }

  func move(to active: Int) { drain?.submit(WindowUpdate(active: active)) }

  func setEnabled(_ enabled: Bool) { drain?.submit(WindowUpdate(isActive: enabled)) }

  func close() {
    generation += 1
    if isReady {
      isReady = false
      report?(false)
    }
    read = nil
    box = nil
    drain?.submit(WindowUpdate(closes: true))
    drain = nil
  }

  private func observe(_ expected: Int) {
    guard expected == generation, let read else { return }
    let latch = latch
    let rows = withObservationTracking { read() } onChange: { [weak self] in
      guard latch.raise() else { return }
      Task { @MainActor in self?.changed(expected, latch) }
    }
    drain?.submit(WindowUpdate(rows: rows))
  }

  private func changed(_ expected: Int, _ latch: ChangeLatch) {
    latch.clear()
    observe(expected)
  }

  private nonisolated static func apply(
    _ update: WindowUpdate<Row>, places: MediaPlaces, layout: WindowLayout<Row>, opened: OpenedWindow, relay: ReadyRelay?,
    failed: @Sendable (MediaError) -> Void
  ) {
    guard !opened.isClosed else { return }
    if update.closes {
      opened.isClosed = true
      opened.window?.release()
      opened.window = nil
      return
    }
    do {
      if opened.window == nil {
        switch layout {
        case .video: opened.window = try places.mountVideoWindow()
        case .image(let name, let rowsPerPage, let reversed, _):
          opened.window = try places.mountImageWindow(name: name, rowsPerPage: rowsPerPage, reversed: reversed)
        }
        if let relay { try opened.window?.subscribeReady(listener: relay) }
      }
      guard let window = opened.window else { return }
      if let active = update.active { opened.active = active }
      let index = UInt32(clamping: max(opened.active, 0))
      switch layout {
      case .video(let source):
        if let rows = update.rows {
          try window.replace(active: index, sources: rows.map(source))
        } else if update.active != nil {
          try window.moveTo(active: index)
        }
      case .image(_, _, _, let sources):
        if let rows = update.rows { try window.replaceRows(rows: rows.map(sources)) }
        if let visible = update.visible {
          try window.showRows(first: UInt32(clamping: max(visible.lowerBound, 0)), last: UInt32(clamping: max(visible.upperBound, 0)))
        } else if update.active != nil {
          try window.moveTo(active: index)
        }
      }
      if let isActive = update.isActive { try window.setActive(isActive: isActive) }
    } catch {
      failed(mediaError(error))
    }
  }
}

private enum WindowPlacement: Equatable {
  case page(Int)
  case visibleRows(name: String)

  var page: Int? {
    if case .page(let page) = self { return page }
    return nil
  }

  var tracksRows: Bool { page == nil }
}

@MainActor
final class VisibleRows {
  private var rows = Set<Int>()
  private var follow: ((ClosedRange<Int>) -> Void)?

  func attach(_ follow: @escaping (ClosedRange<Int>) -> Void) {
    self.follow = follow
    publish()
  }

  func appeared(_ row: Int) {
    rows.insert(row)
    publish()
  }

  func disappeared(_ row: Int) {
    rows.remove(row)
    publish()
  }

  private func publish() {
    guard let first = rows.min(), let last = rows.max() else { return }
    follow?(first...last)
  }
}

private extension EnvironmentValues {
  @Entry var mediaWindowRows: [String: VisibleRows] = [:]
}

private struct WindowModifier<Row: Sendable>: ViewModifier {
  @ResolvedMediaContext private var context
  @State private var follower = WindowFollower<Row>()
  @State private var visibleRows = VisibleRows()
  let layout: WindowLayout<Row>
  let identity: AnyHashable
  let data: @MainActor () -> [Row]
  let placement: WindowPlacement
  let enabled: Bool
  let ready: Binding<Bool>?
  let failed: @Sendable (MediaError) -> Void

  func body(content: Content) -> some View {
    follower.use(layout: layout)
    follower.use(ready: ready.map { binding in { binding.wrappedValue = $0 } })
    return content
      .transformEnvironment(\.mediaWindowRows) { rows in
        if case .visibleRows(let name) = placement { rows[name] = visibleRows }
      }
      .onAppear { mount() }
      .onDisappear { follower.close() }
      .onChange(of: context.map { ObjectIdentifier($0.places) }) { mount() }
      .onChange(of: identity) { mount() }
      .onChange(of: placement.page) { _, value in
        if let value { follower.move(to: value) }
      }
      .onChange(of: enabled) { _, value in follower.setEnabled(value) }
  }

  private func mount() {
    guard let places = context?.places else {
      follower.close()
      return
    }
    follower.mount(places: places, layout: layout, data: data, active: placement.page ?? 0, enabled: enabled, tracksRows: placement.tracksRows, failed: { failed($0) })
    if placement.tracksRows {
      let follower = follower
      visibleRows.attach { follower.show($0) }
    }
  }
}

private struct WindowRow: ViewModifier {
  let name: String
  let index: Int
  @Environment(\.mediaWindowRows) private var windows

  func body(content: Content) -> some View {
    content
      .onAppear { windows[name]?.appeared(index) }
      .onDisappear { windows[name]?.disappeared(index) }
  }
}

extension View {
  /// Declares the video window of a feed. The window reads `rows` of `model` on the main actor and follows that model;
  /// another model mounts a new window. `source` is a pure function of a row, run off the main thread; the latest one serves the next send.
  /// `ready` is true while the first frame of the active source has been rendered or the source is cached, and false again after every change of `active` or of the active source.
  public func mediaWindow<Model: AnyObject & Observable, Row: Sendable>(
    of model: Model, rows: KeyPath<Model, [Row]>, source: @escaping @Sendable (Row) -> MediaItem?, active: Int, enabled: Bool = true,
    ready: Binding<Bool>? = nil, failed: @escaping @Sendable (MediaError) -> Void = { _ in }
  ) -> some View {
    modifier(WindowModifier(
      layout: .video(source: source), identity: AnyHashable(ObjectIdentifier(model)), data: { model[keyPath: rows] },
      placement: .page(active), enabled: enabled, ready: ready, failed: failed
    ))
  }

  /// Declares a named image window over `rows` of `model`. `sources` is a pure function of a row. `rowsPerPage` neighbouring rows form one page and `active` is a page index.
  /// A `reversed` list counts its pages from the last row. `ready` is true while every source of the active page is cached or failed, and false again after every change of the active page.
  public func mediaImageWindow<Model: AnyObject & Observable, Row: Sendable>(
    name: String, of model: Model, rows: KeyPath<Model, [Row]>, sources: @escaping @Sendable (Row) -> [MediaItem], rowsPerPage: Int = 1,
    reversed: Bool = false, active: Int, enabled: Bool = true, ready: Binding<Bool>? = nil, failed: @escaping @Sendable (MediaError) -> Void = { _ in }
  ) -> some View {
    imageWindow(
      name: name, of: model, rows: rows, sources: sources, rowsPerPage: rowsPerPage, reversed: reversed, placement: .page(active), enabled: enabled, ready: ready,
      failed: failed
    )
  }

  /// Declares a named image window whose active page is the page of the first visible row, the last one in a `reversed` list.
  /// Rows report their place with `mediaWindowRow(_:index:)`. The window does not move while no row is visible.
  public func mediaImageWindow<Model: AnyObject & Observable, Row: Sendable>(
    name: String, of model: Model, rows: KeyPath<Model, [Row]>, sources: @escaping @Sendable (Row) -> [MediaItem], rowsPerPage: Int = 1,
    reversed: Bool = false, enabled: Bool = true, ready: Binding<Bool>? = nil, failed: @escaping @Sendable (MediaError) -> Void = { _ in }
  ) -> some View {
    imageWindow(
      name: name, of: model, rows: rows, sources: sources, rowsPerPage: rowsPerPage, reversed: reversed, placement: .visibleRows(name: name), enabled: enabled,
      ready: ready, failed: failed
    )
  }

  /// Reports that the row `index` of the list is on screen to the `mediaImageWindow` of the same `name` above it.
  public func mediaWindowRow(_ name: String, index: Int) -> some View {
    modifier(WindowRow(name: name, index: index))
  }

  private func imageWindow<Model: AnyObject & Observable, Row: Sendable>(
    name: String, of model: Model, rows: KeyPath<Model, [Row]>, sources: @escaping @Sendable (Row) -> [MediaItem], rowsPerPage: Int, reversed: Bool,
    placement: WindowPlacement, enabled: Bool, ready: Binding<Bool>?, failed: @escaping @Sendable (MediaError) -> Void
  ) -> some View {
    precondition(rowsPerPage > 0, "rowsPerPage must be positive")
    return modifier(WindowModifier(
      layout: .image(name: name, rowsPerPage: rowsPerPage, reversed: reversed, sources: sources),
      identity: AnyHashable(Identity(name: name, rowsPerPage: rowsPerPage, reversed: reversed, model: ObjectIdentifier(model))),
      data: { model[keyPath: rows] }, placement: placement, enabled: enabled, ready: ready, failed: failed
    ))
  }
}

private struct Identity: Hashable {
  let name: String
  let rowsPerPage: Int
  let reversed: Bool
  let model: ObjectIdentifier
}

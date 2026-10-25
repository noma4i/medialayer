import MediaLayer
import Observation
import SwiftUI

enum WindowLayout<Row: Sendable>: Sendable {
  case video(source: @Sendable (Row) -> MediaItem?)
  case image(name: String, rowsPerPage: Int, sources: @Sendable (Row) -> [MediaItem])
}

private struct WindowUpdate<Row: Sendable>: Sendable {
  var rows: [Row]?
  var active: Int?
  var isActive: Bool?
  var closes = false

  static func merged(_ held: WindowUpdate?, _ next: WindowUpdate) -> WindowUpdate {
    WindowUpdate(rows: next.rows ?? held?.rows, active: next.active ?? held?.active, isActive: next.isActive ?? held?.isActive, closes: (held?.closes ?? false) || next.closes)
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

  func mount(
    places: MediaPlaces, layout: WindowLayout<Row>, data: @escaping @MainActor () -> [Row], active: Int, enabled: Bool,
    failed: @escaping @Sendable (MediaError) -> Void
  ) {
    close()
    generation += 1
    latch = ChangeLatch()
    read = data
    let box = WindowLayoutBox(layout)
    self.box = box
    let opened = OpenedWindow()
    drain = LatestDrain(merge: WindowUpdate.merged) { update in
      WindowFollower.apply(update, places: places, layout: box.current, opened: opened, failed: failed)
    }
    drain?.submit(WindowUpdate(active: active, isActive: enabled))
    observe(generation)
  }

  /// A mapper is a pure function of a row. The latest one serves the next send; rows already sent are not mapped again.
  func use(layout: WindowLayout<Row>) { box?.use(layout) }

  func move(to active: Int) { drain?.submit(WindowUpdate(active: active)) }

  func setEnabled(_ enabled: Bool) { drain?.submit(WindowUpdate(isActive: enabled)) }

  func close() {
    generation += 1
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
    _ update: WindowUpdate<Row>, places: MediaPlaces, layout: WindowLayout<Row>, opened: OpenedWindow, failed: @Sendable (MediaError) -> Void
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
        case .image(let name, _, _): opened.window = try places.mountImageWindow(name: name)
        }
      }
      guard let window = opened.window else { return }
      if let active = update.active { opened.active = active }
      let index = UInt32(clamping: max(opened.active, 0))
      if let rows = update.rows {
        switch layout {
        case .video(let source): try window.replace(active: index, sources: rows.map(source))
        case .image(_, let rowsPerPage, let sources): try window.replacePages(active: index, pages: WindowFollower.pages(rows, rowsPerPage, sources))
        }
      } else if update.active != nil {
        try window.moveTo(active: index)
      }
      if let isActive = update.isActive { try window.setActive(isActive: isActive) }
    } catch {
      failed(mediaError(error))
    }
  }

  private nonisolated static func pages(_ rows: [Row], _ size: Int, _ sources: (Row) -> [MediaItem]) -> [[MediaItem]] {
    stride(from: 0, to: rows.count, by: size).map { start in rows[start..<min(start + size, rows.count)].flatMap(sources) }
  }
}

private struct WindowModifier<Row: Sendable>: ViewModifier {
  @Environment(\.mediaContext) private var context
  @State private var follower = WindowFollower<Row>()
  let layout: WindowLayout<Row>
  let identity: AnyHashable
  let data: @MainActor () -> [Row]
  let active: Int
  let enabled: Bool
  let failed: @Sendable (MediaError) -> Void

  func body(content: Content) -> some View {
    follower.use(layout: layout)
    return content
      .onAppear { mount() }
      .onDisappear { follower.close() }
      .onChange(of: context.map { ObjectIdentifier($0.places) }) { mount() }
      .onChange(of: identity) { mount() }
      .onChange(of: active) { _, value in follower.move(to: value) }
      .onChange(of: enabled) { _, value in follower.setEnabled(value) }
  }

  private func mount() {
    guard let places = context?.places else {
      follower.close()
      return
    }
    follower.mount(places: places, layout: layout, data: data, active: active, enabled: enabled, failed: failed)
  }
}

extension View {
  /// Declares the video window of a feed. The window reads `rows` of `model` on the main actor and follows that model;
  /// another model mounts a new window. `source` is a pure function of a row, run off the main thread; the latest one serves the next send.
  public func mediaWindow<Model: AnyObject & Observable, Row: Sendable>(
    of model: Model, rows: KeyPath<Model, [Row]>, source: @escaping @Sendable (Row) -> MediaItem?, active: Int, enabled: Bool = true,
    failed: @escaping @Sendable (MediaError) -> Void = { _ in }
  ) -> some View {
    modifier(WindowModifier(layout: .video(source: source), identity: AnyHashable(ObjectIdentifier(model)), data: { model[keyPath: rows] }, active: active, enabled: enabled, failed: failed))
  }

  /// Declares a named image window over `rows` of `model`. `sources` is a pure function of a row. `rowsPerPage` neighbouring rows form one page and `active` is a page index.
  public func mediaImageWindow<Model: AnyObject & Observable, Row: Sendable>(
    name: String, of model: Model, rows: KeyPath<Model, [Row]>, sources: @escaping @Sendable (Row) -> [MediaItem], rowsPerPage: Int = 1,
    active: Int, enabled: Bool = true, failed: @escaping @Sendable (MediaError) -> Void = { _ in }
  ) -> some View {
    precondition(rowsPerPage > 0, "rowsPerPage must be positive")
    return modifier(WindowModifier(
      layout: .image(name: name, rowsPerPage: rowsPerPage, sources: sources), identity: AnyHashable(Identity(name: name, rowsPerPage: rowsPerPage, model: ObjectIdentifier(model))),
      data: { model[keyPath: rows] }, active: active, enabled: enabled, failed: failed
    ))
  }
}

private struct Identity: Hashable {
  let name: String
  let rowsPerPage: Int
  let model: ObjectIdentifier
}

import SwiftUI

#if canImport(UIKit)
import UIKit

private final class VisibilityPass: NSObject {
  static let shared = VisibilityPass()
  private let pending = NSHashTable<OnScreenView>.weakObjects()
  private var displayLink: CADisplayLink?

  func schedule(_ view: OnScreenView) {
    pending.add(view)
    guard displayLink == nil else { return }
    let link = CADisplayLink(target: self, selector: #selector(flush))
    displayLink = link
    link.add(to: .main, forMode: .common)
  }

  func cancel(_ view: OnScreenView) { pending.remove(view) }

  @objc private func flush() {
    displayLink?.invalidate()
    displayLink = nil
    let views = pending.allObjects
    pending.removeAllObjects()
    var clips: [ObjectIdentifier: CGRect] = [:]
    func clip(_ view: UIView, window: UIWindow) -> CGRect {
      let key = ObjectIdentifier(view)
      if let cached = clips[key] { return cached }
      let rect: CGRect
      if view.isHidden || view.alpha == 0 { rect = .null }
      else if let parent = view.superview {
        let inherited = clip(parent, window: window)
        rect = view.clipsToBounds || view is UIScrollView ? inherited.intersection(view.convert(view.bounds, to: window)) : inherited
      } else { rect = window.bounds }
      clips[key] = rect
      return rect
    }
    for view in views {
      guard let window = view.window else { view.deliver(false); continue }
      let visible = clip(view, window: window).intersection(view.convert(view.bounds, to: window))
      view.deliver(!visible.isNull && !visible.isEmpty)
    }
  }
}

private final class VisibilityObservation {
  static let entries = NSMapTable<UIView, VisibilityObservation>(keyOptions: .weakMemory, valueOptions: .strongMemory)
  private let views = NSHashTable<OnScreenView>.weakObjects()
  private var tokens: [NSKeyValueObservation] = []
  private weak var ancestor: UIView?

  static func join(_ ancestor: UIView, view: OnScreenView) -> VisibilityObservation {
    let observation = entries.object(forKey: ancestor) ?? VisibilityObservation()
    entries.setObject(observation, forKey: ancestor)
    observation.views.add(view)
    if observation.tokens.isEmpty {
      observation.ancestor = ancestor
      observation.tokens.append(ancestor.observe(\.isHidden) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(ancestor.observe(\.alpha) { [weak observation] _, _ in observation?.schedule() })
      let layer = ancestor.layer
      observation.tokens.append(layer.observe(\.opacity) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.position) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.bounds) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.transform) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.sublayerTransform) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.masksToBounds) { [weak observation] _, _ in observation?.schedule() })
      observation.tokens.append(layer.observe(\.isHidden) { [weak observation] _, _ in observation?.schedule() })
      if let scroll = ancestor as? UIScrollView {
        observation.tokens.append(scroll.observe(\.contentOffset) { [weak observation] _, _ in observation?.schedule() })
      }
    }
    return observation
  }

  func leave(_ view: OnScreenView) {
    views.remove(view)
    if views.allObjects.isEmpty {
      tokens = []
      if let ancestor { Self.entries.removeObject(forKey: ancestor) }
    }
  }

  private func schedule() {
    views.allObjects.forEach { VisibilityPass.shared.schedule($0) }
  }
}

final class OnScreenView: UIView {
  var onChange: (Bool) -> Void = { _ in }
  var onDismantle: () -> Void = {}
  private var delivered = false
  private var observations: [VisibilityObservation] = []

  override func didMoveToWindow() {
    super.didMoveToWindow()
    observations.forEach { $0.leave(self) }
    observations = []
    var ancestor: UIView? = self
    while let view = ancestor {
      observations.append(VisibilityObservation.join(view, view: self))
      ancestor = view.superview
    }
    report()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    report()
  }

  func finish() {
    VisibilityPass.shared.cancel(self)
    observations.forEach { $0.leave(self) }
    observations = []
    delivered = false
    onChange = { _ in }
    let release = onDismantle
    onDismantle = {}
    release()
  }

  private func report() {
    VisibilityPass.shared.schedule(self)
  }

  fileprivate func deliver(_ on: Bool) {
    guard on != delivered else { return }
    delivered = on
    onChange(on)
  }
}

struct OnScreenProbe: UIViewRepresentable {
  let onChange: (Bool) -> Void
  var onDismantle: () -> Void = {}

  func makeUIView(context: Context) -> OnScreenView {
    let view = OnScreenView()
    view.isUserInteractionEnabled = false
    view.onChange = onChange
    view.onDismantle = onDismantle
    return view
  }

  func updateUIView(_ view: OnScreenView, context: Context) {
    view.onChange = onChange
    view.onDismantle = onDismantle
  }

  static func dismantleUIView(_ view: OnScreenView, coordinator: ()) {
    view.finish()
  }
}
#else
import AppKit

final class OnScreenView: NSView {
  var onChange: (Bool) -> Void = { _ in }
  var onDismantle: () -> Void = {}
  private var delivered = false
  private var scheduled = false
  private var tokens: [NSObjectProtocol] = []
  private var visibility: NSKeyValueObservation?

  var isOnScreen: Bool {
    guard let window, window.isVisible, !window.isMiniaturized, !isHiddenOrHasHiddenAncestor else { return false }

    return !visibleRect.intersection(bounds).isEmpty
  }

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    watch()
    report()
  }

  override func setFrameSize(_ newSize: NSSize) {
    super.setFrameSize(newSize)
    report()
  }

  override func viewDidHide() {
    super.viewDidHide()
    report()
  }

  override func viewDidUnhide() {
    super.viewDidUnhide()
    report()
  }

  func finish() {
    unwatch()
    delivered = false
    onChange = { _ in }
    let release = onDismantle
    onDismantle = {}
    release()
  }

  private func watch() {
    unwatch()
    guard let window else { return }

    let center = NotificationCenter.default
    var clips: [NSClipView] = []
    var ancestor = superview
    while let view = ancestor {
      if let clip = view as? NSClipView { clips.append(clip) }
      ancestor = view.superview
    }
    for clip in clips {
      clip.postsBoundsChangedNotifications = true
      tokens.append(center.addObserver(forName: NSView.boundsDidChangeNotification, object: clip, queue: .main) { [weak self] _ in self?.report() })
    }
    for name in [NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification] {
      tokens.append(center.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in self?.report() })
    }
    visibility = window.observe(\.isVisible) { [weak self] _, _ in self?.report() }
  }

  private func unwatch() {
    tokens.forEach { NotificationCenter.default.removeObserver($0) }
    tokens = []
    visibility = nil
  }

  private func report() {
    guard !scheduled else { return }

    scheduled = true
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }

      scheduled = false
      let on = isOnScreen
      guard on != delivered else { return }

      delivered = on
      onChange(on)
    }
  }
}

struct OnScreenProbe: NSViewRepresentable {
  let onChange: (Bool) -> Void
  var onDismantle: () -> Void = {}

  func makeNSView(context: Context) -> OnScreenView {
    let view = OnScreenView()
    view.onChange = onChange
    view.onDismantle = onDismantle
    return view
  }

  func updateNSView(_ view: OnScreenView, context: Context) {
    view.onChange = onChange
    view.onDismantle = onDismantle
  }

  static func dismantleNSView(_ view: OnScreenView, coordinator: ()) {
    view.finish()
  }
}
#endif

import SwiftUI

#if canImport(UIKit)
import UIKit

final class OnScreenView: UIView {
  var onChange: (Bool) -> Void = { _ in }
  private var delivered = false
  private var scheduled = false
  private var observations: [NSKeyValueObservation] = []

  var isOnScreen: Bool {
    guard let window, !window.isHidden else { return false }

    var rect = bounds
    var view: UIView = self
    while let parent = view.superview {
      if parent.isHidden { return false }

      rect = view.convert(rect, to: parent)
      if parent.clipsToBounds || parent is UIScrollView { rect = rect.intersection(parent.bounds) }
      if rect.isNull || rect.isEmpty { return false }

      view = parent
    }
    return true
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    observations = []
    if let window { observations.append(window.observe(\.isHidden) { [weak self] _, _ in self?.report() }) }
    var ancestor: UIView? = self
    while let view = ancestor {
      observations.append(view.observe(\.isHidden) { [weak self] _, _ in self?.report() })
      if let scroll = view as? UIScrollView { observations.append(scroll.observe(\.contentOffset) { [weak self] _, _ in self?.report() }) }
      ancestor = view.superview
    }
    report()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    report()
  }

  func finish() {
    observations = []
    guard delivered else { return }

    delivered = false
    onChange(false)
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

struct OnScreenProbe: UIViewRepresentable {
  let onChange: (Bool) -> Void

  func makeUIView(context: Context) -> OnScreenView {
    let view = OnScreenView()
    view.isUserInteractionEnabled = false
    view.onChange = onChange
    return view
  }

  func updateUIView(_ view: OnScreenView, context: Context) {
    view.onChange = onChange
  }

  static func dismantleUIView(_ view: OnScreenView, coordinator: ()) {
    view.finish()
  }
}
#else
import AppKit

final class OnScreenView: NSView {
  var onChange: (Bool) -> Void = { _ in }
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
    guard delivered else { return }

    delivered = false
    onChange(false)
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

  func makeNSView(context: Context) -> OnScreenView {
    let view = OnScreenView()
    view.onChange = onChange
    return view
  }

  func updateNSView(_ view: OnScreenView, context: Context) {
    view.onChange = onChange
  }

  static func dismantleNSView(_ view: OnScreenView, coordinator: ()) {
    view.finish()
  }
}
#endif

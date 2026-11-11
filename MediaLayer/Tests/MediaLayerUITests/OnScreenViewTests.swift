#if canImport(UIKit)
import Testing
import UIKit
@testable import MediaLayerUI

private final class ObservedScroll: UIScrollView {
  var offsetObservers = 0

  override func addObserver(_ observer: NSObject, forKeyPath keyPath: String, options: NSKeyValueObservingOptions = [], context: UnsafeMutableRawPointer?) {
    if keyPath == "contentOffset" { offsetObservers += 1 }
    super.addObserver(observer, forKeyPath: keyPath, options: options, context: context)
  }
}

@MainActor
private func settle() async {
  let end = Date().addingTimeInterval(0.3)
  while Date() < end {
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private func settle(until holds: () -> Bool) async {
  let end = Date().addingTimeInterval(10)
  while !holds(), Date() < end {
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct OnScreenViewTests {
  @Test
  func inv_visibility_01_views_share_one_scroll_observer() async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let scroll = ObservedScroll(frame: window.bounds)
    scroll.contentSize = CGSize(width: 200, height: 2000)
    window.addSubview(scroll)
    let before = scroll.offsetObservers
    let probes = (0 ..< 3).map { index in
      let probe = OnScreenView(frame: CGRect(x: index * 50, y: 0, width: 50, height: 50))
      scroll.addSubview(probe)
      return probe
    }
    await settle(until: { scroll.offsetObservers - before == 1 })
    #expect(scroll.offsetObservers - before == 1)
    probes.forEach { $0.finish() }
    window.isHidden = true
  }

  @Test
  func a_view_scrolled_out_of_its_scroll_view_reports_off_screen_and_reports_on_screen_when_scrolled_back() async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let scroll = UIScrollView(frame: window.bounds)
    scroll.contentSize = CGSize(width: 200, height: 2000)
    let probe = OnScreenView(frame: CGRect(x: 0, y: 1900, width: 50, height: 50))
    var reports: [Bool] = []
    probe.onChange = { reports.append($0) }
    scroll.addSubview(probe)
    window.addSubview(scroll)
    await settle()
    #expect(reports == [])

    scroll.contentOffset = CGPoint(x: 0, y: 1850)
    await settle(until: { reports == [true] })
    #expect(reports == [true])

    scroll.contentOffset = .zero
    await settle(until: { reports == [true, false] })
    #expect(reports == [true, false])
  }

  @Test
  func a_view_that_leaves_its_window_reports_off_screen() async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let probe = OnScreenView(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
    var reports: [Bool] = []
    probe.onChange = { reports.append($0) }
    window.addSubview(probe)
    await settle(until: { reports == [true] })
    #expect(reports == [true])

    probe.removeFromSuperview()
    await settle(until: { reports == [true, false] })
    #expect(reports == [true, false])
  }
}
enum MoveWithoutLayout: String, CaseIterable, CustomTestStringConvertible {
  case frameOrigin, layerPosition, boundsOrigin, transform, clipsToBounds

  var testDescription: String { rawValue }
}

enum HideWithoutLayout: String, CaseIterable, CustomTestStringConvertible {
  case viewHidden, layerHidden, alpha, layerOpacity, frameOrigin, layerPosition, boundsOrigin, transform, clipsToBounds, windowHidden

  var testDescription: String { rawValue }
}

extension OnScreenViewTests {
  @Test(arguments: MoveWithoutLayout.allCases)
  func inv_visibility_03_an_ancestor_move_without_layout_reports_on_screen(_ move: MoveWithoutLayout) async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let clip = UIView(frame: window.bounds)
    clip.clipsToBounds = true
    let mover = UIView(frame: CGRect(x: 300, y: 0, width: 100, height: 100))
    let probe = OnScreenView(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
    if move == .clipsToBounds {
      clip.frame = CGRect(x: 0, y: 0, width: 10, height: 10)
      mover.frame = CGRect(x: 100, y: 50, width: 100, height: 50)
    }
    var reports: [Bool] = []
    probe.onChange = { reports.append($0) }
    mover.addSubview(probe)
    clip.addSubview(mover)
    window.addSubview(clip)
    await settle()
    #expect(reports == [])

    switch move {
    case .frameOrigin: mover.frame.origin.x = 0
    case .layerPosition: mover.layer.position = CGPoint(x: 50, y: 50)
    case .boundsOrigin: mover.bounds.origin.x = 300
    case .transform: mover.transform = CGAffineTransform(translationX: -300, y: 0)
    case .clipsToBounds: clip.clipsToBounds = false
    }
    await settle(until: { reports == [true] })
    #expect(reports == [true])
    probe.finish()
    window.isHidden = true
  }

  @Test(arguments: HideWithoutLayout.allCases)
  func inv_visibility_04_a_hidden_subtree_reports_off_screen_without_layout(_ hide: HideWithoutLayout) async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let clip = UIView(frame: window.bounds)
    let parent = UIView(frame: window.bounds)
    let probe = OnScreenView(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
    var reports: [Bool] = []
    probe.onChange = { reports.append($0) }
    parent.addSubview(probe)
    clip.addSubview(parent)
    window.addSubview(clip)
    await settle(until: { reports == [true] })
    #expect(reports == [true])

    switch hide {
    case .viewHidden: parent.isHidden = true
    case .layerHidden: parent.layer.isHidden = true
    case .alpha: parent.alpha = 0
    case .layerOpacity: parent.layer.opacity = 0
    case .frameOrigin: parent.frame.origin.x = 500
    case .layerPosition: parent.layer.position = CGPoint(x: 700, y: 50)
    case .boundsOrigin: parent.bounds.origin.x = 500
    case .transform: parent.transform = CGAffineTransform(translationX: 500, y: 0)
    case .clipsToBounds:
      clip.clipsToBounds = true
      clip.frame = CGRect(x: 0, y: 0, width: 1, height: 1)
      parent.frame = CGRect(x: 100, y: 50, width: 100, height: 50)
      probe.frame = CGRect(x: 0, y: 0, width: 50, height: 50)
    case .windowHidden: window.isHidden = true
    }
    await settle(until: { reports == [true, false] })
    #expect(reports == [true, false])
    probe.finish()
    window.isHidden = true
  }

  @Test
  func inv_visibility_04_an_ancestor_shown_again_reports_on_screen() async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let parent = UIView(frame: window.bounds)
    parent.alpha = 0
    let probe = OnScreenView(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
    var reports: [Bool] = []
    probe.onChange = { reports.append($0) }
    parent.addSubview(probe)
    window.addSubview(parent)
    await settle()
    #expect(reports == [])

    parent.alpha = 1
    await settle(until: { reports == [true] })
    #expect(reports == [true])
    probe.finish()
    window.isHidden = true
  }

  // mutation: VisibilityObservation.schedule runs a pass at once or queues each observed view per contentOffset change instead of one flush per display frame
  @Test
  func inv_visibility_05_offset_changes_within_one_frame_run_one_pass_over_each_view_and_a_quiet_frame_runs_none() async {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
    window.isHidden = false
    let scroll = UIScrollView(frame: window.bounds)
    scroll.contentSize = CGSize(width: 200, height: 2000)
    window.addSubview(scroll)
    let probes = (0 ..< 4).map { index in
      let probe = OnScreenView(frame: CGRect(x: 0, y: index * 60, width: 50, height: 50))
      scroll.addSubview(probe)
      return probe
    }
    window.layoutIfNeeded()
    let pass = VisibilityPass.shared
    let displayFrame = NSSelectorFromString("flush")
    _ = pass.perform(displayFrame)
    let passes = pass.passes
    let evaluations = pass.evaluations

    for step in 1 ... 5 { scroll.contentOffset = CGPoint(x: 0, y: step * 10) }
    window.layoutIfNeeded()
    #expect(pass.passes == passes)
    _ = pass.perform(displayFrame)
    #expect(pass.passes == passes + 1)
    #expect(pass.evaluations == evaluations + probes.count)

    await displayFrames(3)
    #expect(pass.passes == passes + 1)
    probes.forEach { $0.finish() }
    window.isHidden = true
  }
}
#endif

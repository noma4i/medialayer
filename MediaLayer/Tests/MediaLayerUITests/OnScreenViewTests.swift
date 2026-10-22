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
    await settle()
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
    await settle()
    #expect(reports == [true])

    scroll.contentOffset = .zero
    await settle()
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
    await settle()
    #expect(reports == [true])

    probe.removeFromSuperview()
    await settle()
    #expect(reports == [true, false])
  }
}
#endif

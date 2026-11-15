#if canImport(UIKit)
import UIKit

@MainActor
func displayFrames(_ count: Int) async {
  let counter = DisplayFrameCounter()
  await eventually("\(count) display frames passed") { counter.ticks >= count }
  counter.stop()
}

private final class DisplayFrameCounter: NSObject {
  private(set) var ticks = 0
  private var link: CADisplayLink?

  override init() {
    super.init()
    let link = CADisplayLink(target: self, selector: #selector(tick))
    link.add(to: .main, forMode: .common)
    self.link = link
  }

  @objc private func tick() { ticks += 1; TestSignal.shared.fire() }

  func stop() {
    link?.invalidate()
    link = nil
  }
}
#endif

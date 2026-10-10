import AVFoundation
import QuartzCore
import Testing
@testable import MediaLayerUI

struct VideoGravityTests {
  @Test
  func hostedPlayerLayerTakesRequestedGravity() {
    let host = CALayer()
    let player = AVPlayerLayer()
    host.addSublayer(player)

    layoutHosted(host, bounds: CGRect(x: 0, y: 0, width: 100, height: 200))
    #expect(player.videoGravity == .resizeAspectFill)

    layoutHosted(host, bounds: CGRect(x: 0, y: 0, width: 100, height: 200), gravity: .resizeAspect)
    #expect(player.videoGravity == .resizeAspect)
    #expect(player.frame == CGRect(x: 0, y: 0, width: 100, height: 200))
  }
}

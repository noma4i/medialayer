#if canImport(UIKit)
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

@MainActor
private final class Deliveries {
  var values: [Bool] = []
}

private struct Probed: View {
  let deliveries: Deliveries

  var body: some View {
    Color.clear.frame(width: 50, height: 50).background(OnScreenProbe { deliveries.values.append($0) })
  }
}

private func find<T: UIView>(_ type: T.Type, in view: UIView) -> T? {
  if let match = view as? T { return match }
  for child in view.subviews {
    if let match = find(type, in: child) { return match }
  }
  return nil
}

struct OnScreenDismantleTests {
  @MainActor
  @Test
  func inv_image_67_aProbeThatIsTornDownDeliversNothingToItsView() async {
    let deliveries = Deliveries()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
    let host = UIHostingController(rootView: AnyView(Probed(deliveries: deliveries)))
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true }
    for _ in 0 ..< 100 where deliveries.values.isEmpty {
      try? await Task.sleep(for: .milliseconds(50))
    }
    #expect(deliveries.values == [true])

    let probe = try? #require(find(OnScreenView.self, in: host.view))
    if let probe { OnScreenProbe.dismantleUIView(probe, coordinator: ()) }

    #expect(deliveries.values == [true])
  }
}
#endif

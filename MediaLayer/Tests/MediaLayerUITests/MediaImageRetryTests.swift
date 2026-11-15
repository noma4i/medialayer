#if canImport(UIKit)
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct RetriedPhoto: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

private struct RetryCell: View {
  let attempt: Int
  let context: MediaContext

  var body: some View {
    MediaImage(RetriedPhoto(url: "file:///cache/picked.png"))
    .id(attempt)
    .environment(\.mediaContext, context)
    .frame(width: 50, height: 50)
  }
}

@MainActor
private func until(_ condition: String, timeout: Double = 5, _ holds: () -> Bool) async {
  let end = Date().addingTimeInterval(timeout)
  while !holds() {
    guard Date() < end else {
      Issue.record("never became true within \(timeout)s: \(condition)")
      return
    }
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(1)))
  struct MediaImageRetryTests {
    @Test
    func inv_image_83_a_failed_image_mounted_again_under_a_new_id_loads_the_same_source_again() async {
      let places = RecordingPlaces()
      ImageMemory.install(ImageCache(), on: places)
      places.imageResult = .failure(MediaError.Fetch(detail: "the file is not there yet"))
      let context = MediaContext(places: places)
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
      let host = UIHostingController(rootView: RetryCell(attempt: 0, context: context))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true; window.rootViewController = nil }

      await until("the first load failed") { places.recorded.contains { $0.hasPrefix("state failed") } }
      places.imageResult = .success(pngData(side: 8))
      host.rootView = RetryCell(attempt: 1, context: context)
      await until("the second load is ready") { places.recorded.contains("state ready") }

      #expect(places.fetchedURLs.count == 2)
    }
  }
}
#endif

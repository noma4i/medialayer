#if canImport(UIKit)
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct NoteSource: MediaSource {
  let url: String
  var kind: Kind { .video }
  var headers: [String: String] { [:] }
}

@MainActor
private final class NoteEvents: ObservableObject {
  @Published var mounted = true
  var log: [String] = [] { didSet { TestSignal.shared.fire() } }
}

private struct NotePage: View {
  let context: MediaContext
  @ObservedObject var events: NoteEvents

  var body: some View {
    if events.mounted {
      MediaVideo(
        NoteSource(url: "file:///note.mp4"),
        loop: false,
        onEnd: {
          events.log.append("end")
          events.mounted = false
        },
        onProgress: { events.log.append("progress \($0)") }
      )
      .environment(\.mediaContext, context)
      .frame(width: 80, height: 80)
    }
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MountedVideoEndTests {
    @Test
    func inv_api_48_aOneShotVideoReportsFullProgressBeforeItsEndEvenWhenTheEndOutrunsEverySample() async {
      let places = RecordingPlaces()
      let events = NoteEvents()
      let host = UIHostingController(rootView: NotePage(context: MediaContext(places: places, slots: LoadedSlots()), events: events))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      let root = UIViewController()
      window.rootViewController = root
      root.addChild(host)
      host.view.frame = CGRect(x: 0, y: 0, width: 80, height: 80)
      host.safeAreaRegions = []
      root.view.addSubview(host.view)
      host.didMove(toParent: root)
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }

      await eventually("the view followed the state of its clip") { places.mainThreadCalls.contains("stateStream") }
      places.emit(.rendered)
      places.emit(.ended)
      await eventually("the consumer removed the view on its end") { events.log.contains("end") }

      let end = events.log.firstIndex(of: "end") ?? 0
      #expect(end > 0 && events.log[end - 1] == "progress 1.0", "the end arrived without a full progress before it: \(events.log)")
    }
  }
}
#endif

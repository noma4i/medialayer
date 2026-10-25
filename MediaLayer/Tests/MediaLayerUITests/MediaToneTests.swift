#if canImport(UIKit)
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct ToneSubject: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

private func tone(_ level: UInt8, band: Float = 0.15) -> MediaTone {
  let swatch = Swatch(color: Rgb(r: level, g: level, b: level), luminance: Double(level) / 255)
  return MediaTone(source: .pixels, columns: 1, rows: 1, cells: [], band: band, overall: swatch, top: swatch, rest: swatch, dominant: [])
}

@MainActor
private final class ToneInputs: ObservableObject {
  @Published var url: String? = "https://o.test/a.jpg"
  @Published var band: Float = 0.15
  private(set) var shown: [MediaTone?] = []
  private(set) var failures: [MediaError?] = []
  var last: MediaTone? { shown.last ?? nil }

  func record(_ tone: MediaTone?, _ error: MediaError?) {
    shown.append(tone)
    failures.append(error)
  }
}

private struct ToneCell: View {
  @ObservedObject var inputs: ToneInputs
  let context: MediaContext

  var body: some View {
    WithMediaTone(source: inputs.url.map { ToneSubject(url: $0) }, band: inputs.band) { tone, error in
      let _ = inputs.record(tone, error)
      Color.clear
    }
    .environment(\.mediaContext, context)
  }
}

@MainActor
private final class ToneScreen {
  let inputs = ToneInputs()
  let places = RecordingPlaces()
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))

  init() {
    window.rootViewController = UIHostingController(rootView: ToneCell(inputs: inputs, context: MediaContext(places: places)))
    window.isHidden = false
  }

  func close() {
    window.isHidden = true
    window.rootViewController = nil
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MediaToneTests {
    @Test
    func inv_tone_20_mounting_subscribes_once_and_unmounting_cancels() async {
      let screen = ToneScreen()
      await eventually("subscribed once") { screen.places.toneRequests.count == 1 }
      let request = screen.places.toneRequests[0]
      #expect(request.source.url == "https://o.test/a.jpg" && request.band == 0.15)
      request.continuation.yield(.tone(tone(40)))
      await eventually("value shown") { screen.inputs.last == tone(40) }
      #expect(screen.places.toneTerminations == 0)

      screen.close()
      await eventually("cancelled on unmount") { screen.places.toneTerminations == 1 }
      #expect(screen.places.toneRequests.count == 1)
    }

    @Test
    func inv_tone_20_a_new_source_replaces_the_subscription_and_clears_the_old_value() async {
      let screen = ToneScreen()
      await eventually("first request") { screen.places.toneRequests.count == 1 }
      screen.places.toneRequests[0].continuation.yield(.tone(tone(40)))
      await eventually("first value") { screen.inputs.last == tone(40) }

      screen.inputs.url = "https://o.test/b.jpg"
      await eventually("second request") { screen.places.toneRequests.count == 2 }
      await eventually("first cancelled") { screen.places.toneTerminations == 1 }
      #expect(screen.places.toneRequests[1].source.url == "https://o.test/b.jpg")
      #expect(screen.inputs.last == nil)

      screen.places.toneRequests[0].continuation.yield(.tone(tone(200)))
      screen.places.toneRequests[1].continuation.yield(.tone(tone(90)))
      await eventually("second value") { screen.inputs.last == tone(90) }
      #expect(!screen.inputs.shown.contains(tone(200)))
      screen.close()
    }

    @Test
    func inv_tone_20_a_nil_source_cancels_and_clears_and_a_band_change_keeps_the_value() async {
      let screen = ToneScreen()
      await eventually("request") { screen.places.toneRequests.count == 1 }
      screen.places.toneRequests[0].continuation.yield(.tone(tone(40)))
      await eventually("value") { screen.inputs.last == tone(40) }

      screen.inputs.band = 0.5
      await eventually("band request") { screen.places.toneRequests.count == 2 }
      #expect(screen.places.toneRequests[1].band == 0.5)
      #expect(screen.inputs.last == tone(40))

      screen.inputs.url = nil
      await eventually("cancelled without source") { screen.places.toneTerminations == 2 }
      await eventually("cleared") { screen.inputs.last == nil }
      #expect(screen.places.toneRequests.count == 2)
      screen.close()
    }

    @Test
    func inv_tone_20_the_platform_keeps_no_table_of_urls_so_a_returning_source_asks_the_core_again() async {
      let screen = ToneScreen()
      await eventually("request a") { screen.places.toneRequests.count == 1 }
      screen.places.toneRequests[0].continuation.yield(.tone(tone(40)))
      await eventually("value a") { screen.inputs.last == tone(40) }

      screen.inputs.url = "https://o.test/b.jpg"
      await eventually("request b") { screen.places.toneRequests.count == 2 }
      screen.inputs.url = "https://o.test/a.jpg"
      await eventually("request a again") { screen.places.toneRequests.count == 3 }
      #expect(screen.inputs.last == nil)
      screen.close()
    }

    @Test
    func inv_tone_20_a_stale_update_of_an_older_generation_is_ignored() async {
      let model = ToneModel()
      let first = model.begin(source: ToneSubject(url: "https://o.test/a.jpg").item)
      let second = model.begin(source: ToneSubject(url: "https://o.test/b.jpg").item)
      model.apply(.tone(tone(200)), generation: first)
      #expect(model.tone == nil)
      model.apply(.tone(tone(90)), generation: second)
      #expect(model.tone == tone(90))
      model.apply(.failed(.Closed), generation: first)
      #expect(model.error == nil)
    }
  }
}
#endif

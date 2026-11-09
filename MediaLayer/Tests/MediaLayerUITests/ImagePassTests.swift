#if canImport(UIKit)
import MediaLayer
import Testing
@testable import MediaLayerUI

@Suite
struct ImagePassTests {
  @Test
  func inv_image_80_the_state_given_to_content_is_ready_in_the_pass_that_draws_the_image() {
    let cases: [(state: MediaState, expected: MediaState)] = [
      (.idle, .ready), (.loading, .ready), (.ready, .ready), (.rendered, .rendered), (.ended, .ended)
    ]
    for (state, expected) in cases {
      #expect(ImagePass.contentState(shown: true, state: state, failure: nil) == expected, "shown, core state \(state)")
    }
    #expect(ImagePass.contentState(shown: true, state: .failed(detail: "core"), failure: nil) == .failed(detail: "core"))
  }

  @Test
  func inv_image_81_the_placeholder_and_the_image_are_never_drawn_in_the_same_pass() {
    for playing in [true, false] {
      for shown in [true, false] {
        let placeholder = ImagePass.drawsPlaceholder(playing: playing, shown: shown)
        #expect(placeholder == (playing && !shown), "playing \(playing), shown \(shown)")
        #expect(!(placeholder && shown))
      }
    }
  }

  @Test
  func inv_image_82_a_failure_wins_and_a_source_that_never_loads_keeps_the_loading_state() {
    let failed = MediaState.failed(detail: "offline")
    #expect(ImagePass.contentState(shown: true, state: .loading, failure: failed) == failed)
    #expect(ImagePass.contentState(shown: false, state: .loading, failure: failed) == failed)
    #expect(ImagePass.contentState(shown: false, state: .loading, failure: nil) == .loading)
    #expect(ImagePass.contentState(shown: false, state: .idle, failure: nil) == .idle)
    #expect(ImagePass.drawsPlaceholder(playing: true, shown: false))
  }
}
#endif

import Foundation
import Testing

@testable import MediaLayerUI

@Suite struct MediaTraceTests {
  private func defaults() -> UserDefaults {
    let name = "MediaTraceTests-\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: name)!
    defaults.removePersistentDomain(forName: name)
    return defaults
  }

  @Test func off_by_default() {
    #expect(!MediaTrace.enabled(environment: [:], defaults: defaults()))
  }

  @Test func on_by_user_default() {
    let defaults = defaults()
    defaults.set(true, forKey: "MLTrace")
    #expect(MediaTrace.enabled(environment: [:], defaults: defaults))
  }

  @Test func on_by_environment() {
    #expect(MediaTrace.enabled(environment: ["ML_TRACE": "1"], defaults: defaults()))
  }

  @Test func off_does_not_evaluate_message() {
    var evaluated = false
    var written = 0
    MediaTrace.emit("src", { evaluated = true; return "x" }(), on: false) { _, _ in written += 1 }
    #expect(!evaluated)
    #expect(written == 0)
  }

  @Test func on_writes_line_with_source() {
    var lines: [(String, String)] = []
    MediaTrace.emit("slot", "hello", on: true) { lines.append(($0, $1)) }
    #expect(lines.count == 1)
    #expect(lines[0].0 == "slot")
    #expect(lines[0].1.hasPrefix("ML "))
    #expect(lines[0].1.hasSuffix(" slot hello"))
  }

  @Test func tail_drops_query_and_fragment_before_the_end() {
    #expect(MediaTrace.tail("https://cdn.example.com/media/clip.mp4?token=SECRETSECRETSECRETSECRET#frag") == "ample.com/media/clip.mp4")
    #expect(MediaTrace.tail("https://cdn.example.com/a/very/long/path/to/clip.mp4") == "y/long/path/to/clip.mp4")
  }

  @Test func error_label_has_no_description() {
    let error = NSError(domain: "d", code: 7, userInfo: [NSLocalizedDescriptionKey: "https://x/y?token=SECRET"])
    #expect(MediaTrace.errorLabel(error) == "d#7")
    #expect(MediaTrace.errorLabel(nil) == "-")
  }
}

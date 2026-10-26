import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private func fixture(_ name: String, _ fileExtension: String) -> MediaSourceValue {
  .file(Bundle.module.url(forResource: name, withExtension: fileExtension, subdirectory: "Fixtures")!, kind: .video)
}

private func open(fetcher: Fetcher = TestFetcher()) async throws -> MediaLayer {
  try await MediaLayer.open(
    fetcher: fetcher,
    settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
    directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  )
}

extension GlobalMedia {
  struct MediaInfoTests {
    @Test
    func inv_info_03_a_clip_turned_by_90_degrees_gives_the_frame_as_the_screen_shows_it() async throws {
      let info = try #require(await open().mediaInfo(fixture("rotated90", "mp4")))

      #expect(info.frame == ImageSize(width: 32, height: 64))
      #expect(abs(Int(try #require(info.durationMs)) - 1500) <= 100)
    }

    @Test
    func inv_info_04_an_audio_file_gives_its_length_and_no_frame() async throws {
      let info = try #require(await open().mediaInfo(fixture("tone", "m4a")))

      #expect(info.frame == nil)
      #expect(abs(Int(try #require(info.durationMs)) - 2000) <= 100)
    }

    @Test
    func inv_info_05_a_file_without_media_gives_nothing_and_a_missing_file_fails() async throws {
      let media = try await open()
      let text = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).txt")
      try Data("not a clip".utf8).write(to: text)
      defer { try? FileManager.default.removeItem(at: text) }

      #expect(try await media.mediaInfo(MediaSourceValue.file(text, kind: .video)) == nil)
      await #expect(throws: MediaError.self) {
        try await media.mediaInfo(MediaSourceValue.file("/tmp/\(UUID().uuidString).mp4", kind: .video))
      }
    }

    @Test
    func inv_info_06_a_remote_source_gives_nothing_without_a_request() async throws {
      let fetcher = TestFetcher()
      let media = try await open(fetcher: fetcher)

      #expect(try await media.mediaInfo(MediaSourceValue.remote("https://o.test/a.mp4", kind: .video)) == nil)
      #expect(fetcher.requests.isEmpty)
    }

    @Test(.timeLimit(.minutes(1)))
    func inv_info_07_reading_does_not_need_the_main_thread() async throws {
      let media = try await open()
      let source = fixture("rotated90", "mp4")
      let gate = DispatchSemaphore(value: 0)
      DispatchQueue.main.async { _ = gate.wait(timeout: .now() + 5) }
      defer { gate.signal() }

      let started = Date()
      let info = try await media.mediaInfo(source)

      #expect(info?.frame == ImageSize(width: 32, height: 64))
      #expect(Date().timeIntervalSince(started) < 3, "the read waited for the blocked main thread")
    }
  }
}

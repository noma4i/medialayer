import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class Expired: @unchecked Sendable {
  private let lock = NSLock()
  private var expired = false

  func set() { lock.withLock { expired = true } }
  var value: Bool { lock.withLock { expired } }
}

private final class NoFetch: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "offline") }
}

struct MediaSourceValueTests {
  @Test
  func inv_source_01_theValueKeepsTheTextAsDeclaredAndTheCoreCleansItAtIntake() {
    let value = MediaSourceValue(url: "  https://o.test/a.jpg\n", kind: .image, cover: "  ", blurHash: " LEHV6nWB ")

    #expect(value.url == "  https://o.test/a.jpg\n")
    #expect(value.cover == "  ")
    #expect(value.blurHash == " LEHV6nWB ")
  }

  @Test
  func inv_source_02_aPathAndAFileUrlBuildOneFileSource() {
    let fromPath = MediaSourceValue.file("/tmp/a b.mp4", kind: .video)
    let fromUrl = MediaSourceValue.file(URL(fileURLWithPath: "/tmp/a b.mp4"), kind: .video)

    #expect(fromPath.url == "/tmp/a b.mp4")
    #expect(fromUrl.url == "file:///tmp/a%20b.mp4")
  }

  @Test
  func inv_source_03_theRemoteFactoryKeepsTheTextAndItsHeaders() {
    #expect(MediaSourceValue.remote(" https://o.test/v.mp4 ", kind: .video).url == " https://o.test/v.mp4 ")
    #expect(MediaSourceValue.remote("ftp://o.test/v.mp4", kind: .video).url == "ftp://o.test/v.mp4")
    #expect(MediaSourceValue.remote("https://o.test/v.mp4", kind: .video, headers: ["X-Token": "a"]).headers == ["X-Token": "a"])
  }

  @Test
  func inv_source_08_buildingASourceKeepsTheTextAsDeclared() {
    #expect(MediaSourceValue.file("clips/a.mp4", kind: .video).url == "clips/a.mp4")
    #expect(MediaSourceValue.remote("not a url", kind: .video).url == "not a url")
    #expect(MediaSourceValue.bundled("missing", extension: "mp4", bundle: .module, kind: .audio).url == "")
    #expect(MediaSourceValue(url: " x ", kind: .video, blurHash: " ").blurHash == " ")
  }

  @Test
  func inv_source_09_aTextThatIsNoSourceUrlReachesItsPlaceAsFailed() async throws {
    let media = try await MediaLayer.open(fetcher: NoFetch(), settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!), directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
    let sources = [
      MediaSourceValue.remote("not a url", kind: .image),
      MediaSourceValue.remote("ftp://o.test/a.jpg", kind: .image),
      MediaSourceValue.file("clips/a.jpg", kind: .image),
    ]
    for source in sources {
      let place = try media.openPlace(item: source.item)
      try place.update(input: PlaceInput(visible: true, playing: false, group: nil, peeking: false, muted: nil, rate: nil, pinned: false, viewToken: nil))
      var last: MediaState?
      let stream = media.stateStream(key: source.url)
      let expired = Expired()
      let bound = Task { try? await Task.sleep(nanoseconds: 5_000_000_000); expired.set(); place.release() }
      for await state in stream {
        last = state
        if state.isFailed || expired.value { break }
      }
      bound.cancel()
      #expect(last == .failed(detail: "the source has no url"), "\(source.url)")
      place.release()
    }
  }

  @Test
  func inv_source_04_aBundledImageBecomesAFileUrlOfImageKind() throws {
    let found = MediaSourceValue.bundled("exif6_editor", extension: "jpg", subdirectory: "Fixtures", bundle: .module, kind: .image)

    #expect(found.url.hasPrefix("file:///"))
    #expect(found.url.hasSuffix("/Fixtures/exif6_editor.jpg"))
    #expect(found.kind == .image)
    #expect(MediaSourceValue.bundled("missing", extension: "jpg", bundle: .module, kind: .image).url == "")
  }

  @Test
  func inv_source_07_aBundleResourceBecomesAFileUrlAndAMissingOneHasNoUrl() throws {
    let found = MediaSourceValue.bundled("silent", extension: "mp4", subdirectory: "Fixtures", bundle: .module, kind: .video)
    let path = try #require(URL(string: found.url)).path

    #expect(found.url.hasPrefix("file:///"))
    #expect(found.url.hasSuffix("/Fixtures/silent.mp4"))
    #expect(FileManager.default.fileExists(atPath: path))
    #expect(found.kind == .video)
    #expect(MediaSourceValue.bundled("missing", extension: "mp4", bundle: .module, kind: .audio).url == "")
  }
}

import Foundation
import MediaLayer
import Testing

struct MediaSourceValueTests {
  @Test
  func inv_source_01_theValueCleansBlankTextThroughTheCore() {
    let value = MediaSourceValue(url: "  https://o.test/a.jpg\n", kind: .image, cover: "  ", blurHash: " LEHV6nWB ")

    #expect(value.url == "https://o.test/a.jpg")
    #expect(value.cover == nil)
    #expect(value.blurHash == "LEHV6nWB")
    #expect(value.item.url == "https://o.test/a.jpg")
    #expect(value.item.cover == nil)
    #expect(MediaSourceValue(url: " \t ", kind: .video).url == "")
  }

  @Test
  func inv_source_02_aPathAndAFileUrlBuildOneFileSource() {
    let fromPath = MediaSourceValue.file("/tmp/a b.mp4", kind: .video)
    let fromUrl = MediaSourceValue.file(URL(fileURLWithPath: "/tmp/a b.mp4"), kind: .video)

    #expect(fromPath.url == "file:///tmp/a%20b.mp4")
    #expect(fromUrl == fromPath)
    #expect(MediaSourceValue.file("clips/a.mp4", kind: .video).url == "")
    #expect(MediaSourceValue.file("https://o.test/a.mp4", kind: .video).url == "")
  }

  @Test
  func inv_source_03_onlyAValidHttpOrHttpsTextIsARemoteSource() {
    #expect(MediaSourceValue.remote(" https://o.test/v.mp4 ", kind: .video).url == "https://o.test/v.mp4")
    #expect(MediaSourceValue.remote(URL(string: "http://o.test/v.mp4")!, kind: .video).url == "http://o.test/v.mp4")
    #expect(MediaSourceValue.remote("ftp://o.test/v.mp4", kind: .video).url == "")
    #expect(MediaSourceValue.remote("not a url", kind: .video).url == "")
    #expect(MediaSourceValue.remote("https://o.test/v.mp4", kind: .video, headers: ["X-Token": "a"]).headers == ["X-Token": "a"])
  }

  @Test
  func inv_source_04_aBundledImageIsRefusedWithTheTextOfTheCore() {
    #expect(bundledSourceError(kind: .image) == "a bundled source is a video or an audio, not an image")
    #expect(bundledSourceError(kind: .video) == nil)
    #expect(bundledSourceError(kind: .audio) == nil)
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

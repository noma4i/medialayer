import Testing
import MediaLayer

struct GeneratedSourcesTests {
  private struct Row {
    let source: any MediaSource
    let item: MediaItem
    let size: MediaSize?
  }

  private func item(_ kind: Kind, _ url: String, _ headers: [String: String], _ cover: String?, _ blurHash: String?, _ strategy: CacheStrategy) -> MediaItem {
    MediaItem(kind: kind, url: url, headers: headers, cover: cover, blurHash: blurHash, strategy: strategy)
  }

  @Test
  func inv_strategy_15_generated_sources_turn_into_media_items_with_the_dsl_values() {
    let auth = ["Authorization": "token"]
    let rows: [Row] = [
      Row(source: RichVideo(url: "https://o.test/v", cover: "https://o.test/c", headers: auth),
          item: item(.video, "https://o.test/v", auth, "https://o.test/c", nil, .whole), size: nil),
      Row(source: RichImage(url: "https://o.test/i", blurHash: "LEHV6n", headers: auth),
          item: item(.image, "https://o.test/i", auth, nil, "LEHV6n", .onDemand), size: MediaSize(width: 320, height: 240)),
      Row(source: RichAudio(url: "https://o.test/a", headers: auth),
          item: item(.audio, "https://o.test/a", auth, nil, nil, .whole), size: nil),
      Row(source: PlainVideo(url: "https://o.test/pv"), item: item(.video, "https://o.test/pv", [:], nil, nil, .window), size: nil),
      Row(source: PlainImage(url: "https://o.test/pi"), item: item(.image, "https://o.test/pi", [:], nil, nil, .window), size: nil),
      Row(source: MaxImage(url: "https://o.test/mi"), item: item(.image, "https://o.test/mi", [:], nil, nil, .window),
          size: MediaSize(width: 2147483647, height: 2147483647)),
      Row(source: PlainAudio(url: "https://o.test/pa"), item: item(.audio, "https://o.test/pa", [:], nil, nil, .window), size: nil)
    ]
    for row in rows {
      #expect(row.source.item == row.item)
      #expect(row.source.size == row.size)
    }
  }

  @Test
  func inv_gen_03_the_embedded_spec_json_is_the_dsl_data() {
    #expect(MediaSpecJSON.value == ExpectedSpecJSON.value)
  }
}

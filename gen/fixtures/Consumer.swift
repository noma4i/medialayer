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

  @Test
  func inv_gen_03_the_generated_spec_reaches_the_core_with_every_dsl_option() throws {
    let parsed = try specDefaults(json: MediaSpecJSON.value)
    let limits = parsed.defaults

    #expect(limits.budget == 3 * 1024 * 1024 * 1024)
    #expect(limits.minBudget == 96 * 1024 * 1024)
    #expect(limits.shrinkTo == 0.75)
    #expect(limits.evict == .expireAfter(seconds: 2 * 86400))
    #expect(limits.memoryBudget == 192 * 1024 * 1024)
    #expect(limits.timeoutSeconds == 180)
    #expect(limits.maxConcurrent == 5)
    #expect(limits.dedupeWindowSeconds == 7200)
    #expect(limits.dedupeMaxEntries == 250)
    #expect(limits.prefetchBudgetMib == 80)
    #expect(limits.slots == 4)
    #expect(limits.prefetchFrom == 2)
    #expect(limits.prefetchTo == 5)
    #expect(limits.leadingChunks == [2, 1])
    #expect(limits.playbackChunksAhead == 4)
    #expect(parsed.pool == PoolSettings(reach: 2, holdOffMs: 7_000, trimDelayMs: 60_000, lowRamSlots: 1, lowRamReach: 0))
    #expect(parsed.namespaceHeaders == ["Authorization", "Cookie"])
    #expect(parsed.forwardHeaders == ["Authorization", "Cookie"])
    #expect(parsed.evictVideo == .lfu)
    #expect(parsed.evictImage == .largestFirst)
    #expect(parsed.evictAudio == .fifo)
    #expect(parsed.imageWindows == [ImageWindow(name: "rich_image", behind: 3, from: 2, to: 6)])
  }
}

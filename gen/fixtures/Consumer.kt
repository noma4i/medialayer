package app.media

import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.toItem
import org.junit.Assert.assertEquals
import org.junit.Test

class GeneratedSourcesTest {
  private class Row(val source: MediaSource, val item: MediaItem, val size: Pair<Int, Int>?)

  @Test
  fun inv_strategy_15_generated_sources_turn_into_media_items_with_the_dsl_values() {
    val auth = mapOf("Authorization" to "token")
    val rows = listOf(
      Row(RichVideo(url = "https://o.test/v", cover = "https://o.test/c", headers = auth),
        MediaItem(Kind.VIDEO, "https://o.test/v", auth, "https://o.test/c", null, strategy = CacheStrategy.WHOLE), null),
      Row(RichImage(url = "https://o.test/i", blurHash = "LEHV6n", headers = auth),
        MediaItem(Kind.IMAGE, "https://o.test/i", auth, null, "LEHV6n", strategy = CacheStrategy.ON_DEMAND), Pair(320, 240)),
      Row(RichAudio(url = "https://o.test/a", headers = auth),
        MediaItem(Kind.AUDIO, "https://o.test/a", auth, null, null, strategy = CacheStrategy.WHOLE), null),
      Row(PlainVideo(url = "https://o.test/pv"), MediaItem(Kind.VIDEO, "https://o.test/pv", emptyMap(), null, null, strategy = CacheStrategy.WINDOW), null),
      Row(PlainImage(url = "https://o.test/pi"), MediaItem(Kind.IMAGE, "https://o.test/pi", emptyMap(), null, null, strategy = CacheStrategy.WINDOW), null),
      Row(MaxImage(url = "https://o.test/mi"), MediaItem(Kind.IMAGE, "https://o.test/mi", emptyMap(), null, null, strategy = CacheStrategy.WINDOW), Pair(Int.MAX_VALUE, Int.MAX_VALUE)),
      Row(PlainAudio(url = "https://o.test/pa"), MediaItem(Kind.AUDIO, "https://o.test/pa", emptyMap(), null, null, strategy = CacheStrategy.WINDOW), null)
    )
    for (row in rows) {
      assertEquals(row.item, row.source.toItem())
      assertEquals(row.size, row.source.size)
    }
  }

  @Test
  fun inv_gen_03_the_embedded_spec_json_is_the_dsl_data() {
    assertEquals(ExpectedSpecJson.VALUE, MediaSpecJson.VALUE)
  }
}

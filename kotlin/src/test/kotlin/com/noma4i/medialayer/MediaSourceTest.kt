package com.noma4i.medialayer

import org.junit.Assert.assertEquals
import org.junit.Test

class MediaSourceTest {
  private data class Clip(override val url: String, override val cover: String?) : MediaSource {
    override val kind: Kind get() = Kind.VIDEO
    override val headers: Map<String, String> get() = mapOf("Authorization" to "token")
  }

  private data class Plain(override val url: String) : MediaSource {
    override val kind: Kind get() = Kind.IMAGE
    override val headers: Map<String, String> get() = emptyMap()
  }

  @Test
  fun aSourceBecomesTheCoreItemWithItsCoverAndBlurHashWhenDeclared() {
    assertEquals(
      MediaItem(Kind.VIDEO, "https://o.test/a.mp4", mapOf("Authorization" to "token"), "https://o.test/c.jpg", null),
      Clip("https://o.test/a.mp4", "https://o.test/c.jpg").toItem()
    )
    assertEquals(MediaItem(Kind.IMAGE, "https://o.test/p.png", emptyMap(), null, null), Plain("https://o.test/p.png").toItem())
  }
}

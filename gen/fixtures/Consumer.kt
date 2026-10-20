package app.media

import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.toItem

fun consumeGeneratedSources(): List<MediaItem> {
    val sources: List<MediaSource> = listOf(
        MomentVideo(url = "https://example.test/video", cover = "https://example.test/cover", headers = mapOf("Authorization" to "value")),
        Avatar(url = "https://example.test/avatar", blurHash = "hash"),
        VoiceNote(url = "https://example.test/audio"),
        WholeClip(url = "https://example.test/whole"),
        MaxImage(url = "https://example.test/max"),
        WholeVoice(url = "https://example.test/voice")
    )
    val size: Pair<Int, Int>? = sources[4].size
    val strategy: CacheStrategy = sources[3].strategy
    val json: String = MediaSpecJson.VALUE
    check(size != null && strategy == CacheStrategy.WHOLE && json.isNotEmpty())
    return sources.map { it.toItem() }
}

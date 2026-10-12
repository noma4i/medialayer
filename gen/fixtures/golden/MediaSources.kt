package app.media

import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource

data class MomentVideo(
    override val url: String,
    override val cover: String? = null,
    override val headers: Map<String, String> = emptyMap(),
) : MediaSource {
    override val kind: Kind get() = Kind.VIDEO
}

data class Avatar(
    override val url: String,
    override val blurHash: String? = null,
    override val headers: Map<String, String> = emptyMap(),
) : MediaSource {
    override val kind: Kind get() = Kind.IMAGE
    override val size: Pair<Int, Int>? get() = Pair(288, 288)
}

data class VoiceNote(
    override val url: String,
    override val headers: Map<String, String> = emptyMap(),
) : MediaSource {
    override val kind: Kind get() = Kind.AUDIO
}

object MediaSpecJson {
    const val VALUE: String = """{"cache":{"budget":2147483648,"min":67108864,"shrink_to":90,"evict":"lru","memory":268435456,"timeout":30,"max_concurrent":2,"dedupe_window":60,"dedupe_entries":500,"prefetch_budget_mib":48,"namespace_headers":["Authorization"]},"videos":[{"name":"moment_video","cover_field":"coverUrl","prefetch":{"from":1,"to":3,"chunks":[4,2]},"pool":{"slots":3,"reach":1,"hold_off":1,"trim_delay":30,"low_ram_slots":2,"low_ram_reach":0},"forward_headers":["Authorization","Cookie","User-Agent","Connection","Accept","Accept-Encoding","Accept-Language","Range"],"evict":null}],"images":[{"name":"avatar","blurhash_field":"blurHash","size":[288,288],"prefetch_window":[1,3],"prefetch_behind":2,"evict":null}],"audios":[{"name":"voice_note","evict":"fifo"}]}"""
}

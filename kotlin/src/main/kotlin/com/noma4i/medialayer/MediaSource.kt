package com.noma4i.medialayer

interface MediaSource {
    val url: String
    val kind: Kind
    val headers: Map<String, String>
    val cover: String? get() = null
    val blurHash: String? get() = null
    val size: Pair<Int, Int>? get() = null
    val strategy: CacheStrategy get() = CacheStrategy.WINDOW
}

fun MediaSource.toItem(): MediaItem = MediaItem(kind, url, headers, cover, blurHash, strategy = strategy)

fun MediaLayer.replaceImageWindow(name: String, active: Int, pages: List<List<MediaSource>>) =
    replaceImageWindow(name, active.coerceAtLeast(0).toUInt(), pages.map { page -> page.map { it.toItem() } })

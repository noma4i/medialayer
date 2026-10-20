import MediaLayer

func consumeGeneratedSources() -> [MediaItem] {
    let sources: [any MediaSource] = [
        MomentVideo(url: "https://example.test/video", cover: "https://example.test/cover", headers: ["Authorization": "value"]),
        Avatar(url: "https://example.test/avatar", blurHash: "hash"),
        VoiceNote(url: "https://example.test/audio"),
        WholeClip(url: "https://example.test/whole"),
        MaxImage(url: "https://example.test/max"),
        WholeVoice(url: "https://example.test/voice")
    ]
    let _: MediaSize? = sources[4].size
    let _: CacheStrategy = sources[3].strategy
    let _: String = MediaSpecJSON.value
    return sources.map(\.item)
}

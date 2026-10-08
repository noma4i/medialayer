import MediaLayer

public struct MomentVideo: MediaSource, Sendable {
    public let url: String
    public let cover: String?
    public let headers: [String: String]
    public var kind: Kind { .video }

    public init(url: String, cover: String? = nil, headers: [String: String] = [:]) {
        self.url = url
        self.cover = cover
        self.headers = headers
    }
}

public struct Avatar: MediaSource, Sendable {
    public let url: String
    public let blurHash: String?
    public let headers: [String: String]
    public var kind: Kind { .image }
    public var size: MediaSize? { MediaSize(width: 288, height: 288) }

    public init(url: String, blurHash: String? = nil, headers: [String: String] = [:]) {
        self.url = url
        self.blurHash = blurHash
        self.headers = headers
    }
}

public struct VoiceNote: MediaSource, Sendable {
    public let url: String
    public let headers: [String: String]
    public var kind: Kind { .audio }

    public init(url: String, headers: [String: String] = [:]) {
        self.url = url
        self.headers = headers
    }
}

public enum MediaSpecJSON {
    public static let value = #"""
{"cache":{"budget":2147483648,"min":67108864,"shrink_to":90,"evict":"lru","memory":268435456,"timeout":30,"max_concurrent":2,"dedupe_window":60,"dedupe_entries":500,"namespace_headers":["Authorization"]},"videos":[{"name":"moment_video","cover_field":"coverUrl","prefetch":{"from":1,"to":3,"depth":2},"pool":{"slots":3,"reach":1,"hold_off":1,"trim_delay":30,"low_ram_slots":2,"low_ram_reach":0},"forward_headers":["Authorization","Cookie","User-Agent","Connection","Accept","Accept-Encoding","Accept-Language","Range"],"evict":null}],"images":[{"name":"avatar","blurhash_field":"blurHash","size":[288,288],"prefetch_window":[1,3],"evict":null}],"audios":[{"name":"voice_note","evict":"fifo"}]}
"""#
}

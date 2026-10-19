use medialayer::prefetch::CacheStrategy;
use medialayer::spec::MediaSpec;

pub struct Source<'a> {
    pub name: &'a str,
    pub kind: &'static str,
    pub optional: Option<&'static str>,
    pub size: Option<(u32, u32)>,
    pub strategy: CacheStrategy,
}

pub fn sources(spec: &MediaSpec) -> Vec<Source<'_>> {
    let videos = spec.videos.iter().map(|video| Source {
        name: &video.name,
        kind: "video",
        optional: video.cover_field.as_ref().map(|_| "cover"),
        size: None,
        strategy: video.strategy,
    });
    let images = spec.images.iter().map(|image| Source {
        name: &image.name,
        kind: "image",
        optional: image.blurhash_field.as_ref().map(|_| "blurHash"),
        size: image.size,
        strategy: image.strategy,
    });
    let audios = spec.audios.iter().map(|audio| Source { name: &audio.name, kind: "audio", optional: None, size: None, strategy: audio.strategy });
    videos.chain(images).chain(audios).collect()
}

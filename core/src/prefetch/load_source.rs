use super::loader::{LoadError, Loader};
use super::plan::Plan;
use super::settled::Outcome;
use super::source::{Source, SourceKind};
use crate::hls::{extract_leading_chunks, Chunk, extract_variants, is_manifest_url, is_multivariant, resolve_url};
use crate::limits::LeadingChunks;
use crate::proxy::Priority;
use std::future::Future;
use std::time::Duration;

pub(super) fn is_hls(source: &Source) -> bool {
    source.kind == SourceKind::Video && is_manifest_url(&source.url)
}

pub(super) async fn load_unit(loader: &dyn Loader, source: &Source, timeout: Duration) -> Outcome {
    let loaded = match source.kind {
        SourceKind::Image => bounded(timeout, loader.load(&source.url, &source.headers, Priority::ImagePrefetch)).await,
        SourceKind::Audio | SourceKind::Video => {
            bounded(timeout, loader.load_first_block(&source.url, &source.headers, Priority::Prefetch)).await
        }
    };
    loaded.map_or_else(|outcome| outcome, |_| Outcome::Loaded)
}

pub(super) async fn bounded(timeout: Duration, load: impl Future<Output = Result<u64, LoadError>>) -> Result<u64, Outcome> {
    match tokio::time::timeout(timeout, load).await {
        Err(_) => Err(Outcome::TimedOut),
        Ok(Err(error)) => Err(Outcome::Failed(error.0)),
        Ok(Ok(bytes)) => Ok(bytes),
    }
}

async fn playlist_text(loader: &dyn Loader, source: &Source, url: &str, timeout: Duration) -> Result<String, Outcome> {
    bounded(timeout, loader.load(url, &source.headers, Priority::Prefetch)).await?;
    let bytes = loader.stored(url, &source.headers).await.ok_or_else(|| Outcome::Failed(format!("playlist {url} is not stored")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) async fn read_plan(loader: &dyn Loader, source: &Source, timeout: Duration) -> Result<Plan, Outcome> {
    let mut playlist_url = source.url.clone();
    let mut text = playlist_text(loader, source, &playlist_url, timeout).await?;
    if is_multivariant(&text) {
        let variant = extract_variants(&text)
            .into_iter()
            .next()
            .ok_or_else(|| Outcome::Failed("master playlist lists no variants".to_string()))?;
        playlist_url = resolve_url(&playlist_url, &variant);
        text = playlist_text(loader, source, &playlist_url, timeout).await?;
    }
    let chunks = extract_leading_chunks(&text, LeadingChunks::MAX_CHUNKS)
        .into_iter()
        .map(|chunk| Chunk { segment: resolve_url(&playlist_url, &chunk.segment), init: chunk.init.map(|init| resolve_url(&playlist_url, &init)) })
        .collect();
    Ok(Plan { chunks, headers: source.headers.clone() })
}

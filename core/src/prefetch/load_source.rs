use super::loader::{LoadError, Loader};
use super::settled::Outcome;
use super::source::{Source, SourceKind};
use crate::hls::{extract_leading_segments, extract_variants, is_manifest_url, is_multivariant, resolve_url};
use crate::proxy::Priority;
use std::future::Future;
use std::time::Duration;

pub(super) async fn load_source(loader: &dyn Loader, source: &Source, depth: Duration, timeout: Duration) -> Outcome {
    let loaded = match source.kind {
        SourceKind::Image => whole_file(loader, source, timeout).await,
        SourceKind::Audio => first_block(loader, source, timeout).await,
        SourceKind::Video if is_manifest_url(&source.url) => leading_segments(loader, source, depth, timeout).await,
        SourceKind::Video => first_block(loader, source, timeout).await,
    };
    loaded.map_or_else(|outcome| outcome, |()| Outcome::Loaded)
}

async fn bounded(timeout: Duration, load: impl Future<Output = Result<(), LoadError>>) -> Result<(), Outcome> {
    match tokio::time::timeout(timeout, load).await {
        Err(_) => Err(Outcome::TimedOut),
        Ok(Err(error)) => Err(Outcome::Failed(error.0)),
        Ok(Ok(())) => Ok(()),
    }
}

async fn whole_file(loader: &dyn Loader, source: &Source, timeout: Duration) -> Result<(), Outcome> {
    bounded(timeout, loader.load(&source.url, &source.headers, Priority::Prefetch)).await
}

async fn first_block(loader: &dyn Loader, source: &Source, timeout: Duration) -> Result<(), Outcome> {
    bounded(timeout, loader.load_first_block(&source.url, &source.headers, Priority::Prefetch)).await
}

async fn playlist_text(loader: &dyn Loader, source: &Source, url: &str, timeout: Duration) -> Result<String, Outcome> {
    bounded(timeout, loader.load(url, &source.headers, Priority::Prefetch)).await?;
    let bytes = loader.stored(url, &source.headers).await.ok_or_else(|| Outcome::Failed(format!("playlist {url} is not stored")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn leading_segments(loader: &dyn Loader, source: &Source, depth: Duration, timeout: Duration) -> Result<(), Outcome> {
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
    let leading = extract_leading_segments(&text, depth.as_secs_f64());
    for segment in leading.init_segments.iter().chain(&leading.segments) {
        let url = resolve_url(&playlist_url, segment);
        bounded(timeout, loader.load(&url, &source.headers, Priority::Prefetch)).await?;
    }
    Ok(())
}

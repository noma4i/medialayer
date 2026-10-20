use super::load_source::{bounded, is_hls, read_plan};
use super::loader::Loader;
use super::settled::Outcome;
use super::source::{Source, SourceKind};
use crate::proxy::Priority;
use std::future::Future;
use std::time::Duration;
use tokio::sync::Notify;

const RETRY_WAITS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1000)];

async fn retried<T, Call: Future<Output = Result<T, Outcome>>>(mut call: impl FnMut() -> Call) -> Result<T, Outcome> {
    let mut waits = RETRY_WAITS.into_iter();
    loop {
        let result = call().await;
        if !matches!(result, Err(Outcome::Failed(_) | Outcome::TimedOut)) {
            return result;
        }
        match waits.next() {
            Some(wait) => tokio::time::sleep(wait).await,
            None => return result,
        }
    }
}

pub(super) async fn load_whole(loader: &dyn Loader, source: &Source, timeout: Duration) -> Outcome {
    let loaded = match source.kind {
        SourceKind::Image => retried(|| async {
            let progress = Notify::new();
            bounded(timeout, &progress, loader.load(&source.url, &source.headers, Priority::ImagePrefetch, &progress)).await
        }).await,
        SourceKind::Audio | SourceKind::Video if !is_hls(source) => {
            retried(|| async {
                let progress = Notify::new();
                bounded(timeout, &progress, loader.load_all_blocks(&source.url, &source.headers, Priority::Prefetch, &progress)).await
            }).await
        }
        SourceKind::Audio | SourceKind::Video => load_stream(loader, source, timeout).await,
    };
    match loaded {
        Ok(_) => {
            loader.keep(&source.url, &source.headers).await;
            Outcome::Loaded
        }
        Err(outcome) => outcome,
    }
}

async fn load_stream(loader: &dyn Loader, source: &Source, timeout: Duration) -> Result<u64, Outcome> {
    let plan = retried(|| read_plan(loader, source, timeout, usize::MAX)).await?;
    for url in &plan.playlists {
        loader.keep(url, &source.headers).await;
    }
    let mut bytes = 0;
    let mut loaded_inits = std::collections::HashSet::new();
    let mut parts = Vec::new();
    for chunk in &plan.chunks {
        let init = chunk.init.iter().filter(|init| loaded_inits.insert(init.as_str()));
        for url in init.chain([&chunk.segment]) {
            let size = retried(|| async {
                let progress = Notify::new();
                bounded(timeout, &progress, loader.load(url, &source.headers, Priority::Prefetch, &progress)).await
            }).await?;
            bytes += size;
            parts.push((url, size));
            loader.keep(url, &source.headers).await;
        }
    }
    for (url, size) in parts {
        if loader.stored_len(url, &source.headers).await != Some(size) {
            return Err(Outcome::Failed("whole source is no longer stored".into()));
        }
    }
    for url in &plan.playlists {
        if loader.stored_len(url, &source.headers).await.is_none() {
            return Err(Outcome::Failed("whole playlist is no longer stored".into()));
        }
    }
    Ok(bytes)
}

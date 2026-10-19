use super::load_source::{is_hls, read_plan};
use super::loader::{LoadError, Loader};
use super::settled::Outcome;
use super::source::{Source, SourceKind};
use crate::proxy::Priority;
use std::future::Future;
use std::time::Duration;

const RETRY_WAITS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1000)];

fn failed(error: LoadError) -> Outcome {
    Outcome::Failed(error.0)
}

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
    loader.keep(&source.url, &source.headers).await;
    let loaded = match source.kind {
        SourceKind::Image => retried(|| async { loader.load(&source.url, &source.headers, Priority::ImagePrefetch).await.map_err(failed) }).await,
        SourceKind::Audio | SourceKind::Video if !is_hls(source) => {
            retried(|| async { loader.load_all_blocks(&source.url, &source.headers, Priority::Prefetch).await.map_err(failed) }).await
        }
        SourceKind::Audio | SourceKind::Video => load_stream(loader, source, timeout).await,
    };
    loaded.map_or_else(|outcome| outcome, |_| Outcome::Loaded)
}

async fn load_stream(loader: &dyn Loader, source: &Source, timeout: Duration) -> Result<u64, Outcome> {
    let plan = retried(|| read_plan(loader, source, timeout, usize::MAX)).await?;
    let mut bytes = 0;
    let mut loaded_inits = std::collections::HashSet::new();
    for chunk in &plan.chunks {
        let init = chunk.init.iter().filter(|init| loaded_inits.insert(init.as_str()));
        for url in init.chain([&chunk.segment]) {
            bytes += retried(|| async { loader.load(url, &source.headers, Priority::Prefetch).await.map_err(failed) }).await?;
            loader.keep(url, &source.headers).await;
        }
    }
    Ok(bytes)
}

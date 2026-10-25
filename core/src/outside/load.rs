use crate::api::MediaError;
use crate::cache::{Cache, Kind};
use crate::fetch::{self, FetchError, Request};
use crate::ffi::ForeignFetcher;
use crate::identity::cache_key;
use crate::limits::Limits;
use crate::memory::{bounded, InputLimits};
use crate::proxy::{ImageFetchResult, ImageOrigin};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Builder;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImageLoadRequest {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub dir: String,
    pub cache_bytes: u64,
    pub timeout_ms: u64,
}

#[uniffi::export]
pub fn load_image(request: ImageLoadRequest, fetcher: Arc<dyn crate::ffi::Fetcher>) -> Result<ImageFetchResult, MediaError> {
    let limits = InputLimits::default();
    load_with(request, Arc::new(ForeignFetcher::new(fetcher, limits)), limits)
}

pub(crate) fn load_with(request: ImageLoadRequest, fetcher: Arc<dyn fetch::Fetcher>, limits: InputLimits) -> Result<ImageFetchResult, MediaError> {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
    runtime.block_on(load(request, fetcher, limits))
}

fn cache_limits(cache_bytes: u64) -> Result<Limits, MediaError> {
    let defaults = Limits::default();
    let limits = Limits { budget: cache_bytes, min_budget: defaults.min_budget.min(cache_bytes), ..defaults };
    limits.validate()?;
    Ok(limits)
}

async fn load(request: ImageLoadRequest, fetcher: Arc<dyn fetch::Fetcher>, limits: InputLimits) -> Result<ImageFetchResult, MediaError> {
    let cache = Cache::open(request.dir.as_str(), cache_limits(request.cache_bytes)?, u64::MAX);
    let key = cache_key(&request.url);
    let timeout = Duration::from_millis(request.timeout_ms);
    let fetched = tokio::time::timeout(timeout, async {
        cache.ready().await;
        if let Some(bytes) = cache.get(&key).await {
            return Ok(ImageFetchResult { bytes, source: ImageOrigin::Cache });
        }
        let bytes = download(fetcher.as_ref(), Request { url: request.url.clone(), headers: request.headers.clone(), range: None }, limits).await?;
        Ok::<_, MediaError>(ImageFetchResult { bytes, source: ImageOrigin::Network })
    })
    .await
    .map_err(|_| MediaError::Timeout { detail: format!("no answer within {} ms", request.timeout_ms) })??;
    if fetched.source == ImageOrigin::Network {
        let _ = cache.put(&key, Kind::Image, &fetched.bytes).await;
    }
    cache.flush().await.map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
    Ok(fetched)
}

async fn download(fetcher: &dyn fetch::Fetcher, request: Request, limits: InputLimits) -> Result<Vec<u8>, MediaError> {
    let response = fetcher.fetch(request).await.map_err(failed)?;
    if response.status != 200 {
        return Err(failed(FetchError::Status(response.status)));
    }
    let mut body = response.body;
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk.map_err(failed)?);
        bounded(bytes.len() as u64, limits.response_bytes).map_err(|error| MediaError::Fetch { detail: format!("response of {} bytes exceeds the limit of {}", error.len, error.limit) })?;
    }
    Ok(bytes)
}

fn failed(error: FetchError) -> MediaError {
    match error {
        FetchError::Timeout => MediaError::Timeout { detail: "the fetcher timed out".to_string() },
        other => MediaError::Fetch { detail: other.to_string() },
    }
}

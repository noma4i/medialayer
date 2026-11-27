use super::inner::ProxyShared;
use super::route::{block_key, content_range_total, ByteRange};
use super::slots::Priority;
use crate::fetch::{header_value, FetchError, Request};
use super::serve_reply::*;

pub(super) const ATTEMPTS: usize = 3;

pub(super) struct Probe {
    pub(super) total: Option<u64>,
    pub(super) origin_type: Option<String>,
    pub(super) ignored_range: bool,
}

pub(super) async fn probe_total(inner: &ProxyShared, target: &str, headers: Upstream) -> Result<Probe, Failure> {
    let response = origin_fetch(inner, target, headers, Some((0, Some(0)))).await?;
    let total = content_range_total(&response.headers)
        .or_else(|| {
            let whole = response.status == 200;
            header_value(&response.headers, "content-length").filter(|_| whole).and_then(|value| value.trim().parse().ok())
        });
    let origin_type = header_value(&response.headers, "content-type").map(str::to_string);
    match (response.status, total) {
        (200, total) => Ok(Probe { total, origin_type, ignored_range: true }),
        (206, Some(total)) => Ok(Probe { total: Some(total), origin_type, ignored_range: false }),
        (206, None) => Err(Box::new(text(502, "origin did not report a size"))),
        (status, _) => Err(Box::new(text(status, "origin refused the request"))),
    }
}

pub(super) async fn probe_sized(inner: &ProxyShared, target: &str, headers: Upstream) -> Result<(u64, Option<String>), Failure> {
    let probe = probe_total(inner, target, headers).await?;
    match probe.total {
        Some(total) => Ok((total, probe.origin_type)),
        None => Err(Box::new(text(502, "origin did not report a size"))),
    }
}

pub(super) async fn origin_fetch(
    inner: &ProxyShared,
    target: &str,
    headers: Upstream,
    range: Option<(u64, Option<u64>)>,
) -> Result<crate::fetch::Response, Failure> {
    let Ok(Ok(permit)) = tokio::time::timeout(inner.config.permit_wait, inner.slots.acquire(Priority::Playback)).await else {
        crate::trace!("heal", "capacity permit-timeout");
        return Err(Box::new(text(503, "busy")));
    };
    let request = Request { url: target.to_string(), headers, range };
    crate::trace!("origin", "fetch {} range={range:?}", crate::trace::tail(target));
    match tokio::time::timeout(inner.timeout(), inner.fetcher.fetch(request)).await {
        Err(_) => Err(Box::new(text(502, "origin timed out"))),
        Ok(Err(FetchError::Status(status))) => Err(Box::new(text(status, "origin refused the request"))),
        Ok(Err(error)) => Err(Box::new(text(502, &error.to_string()))),
        Ok(Ok(response)) => Ok(crate::fetch::Response { body: permit.hold_idle(response.body, inner.config.direct_idle), ..response }),
    }
}


pub(super) async fn known_total(
    inner: &ProxyShared,
    target: &str,
    key: &str,
    headers: &Upstream,
    range: Option<ByteRange>,
) -> Result<Option<u64>, Failure> {
    if let Some(total) = inner.known_total(key) {
        return Ok(Some(total));
    }
    if let Some(ByteRange::From { start, .. }) = range {
        let first = block_key(key, start / inner.config.chunk_size);
        if let Some(total) = inner.store.meta(&first).await.and_then(|meta| meta.total_length) {
            inner.remember_total(key, total);
            return Ok(Some(total));
        }
    }
    let lock = inner.probe_lock(key);
    probe_once(inner, target, key, headers, &lock.lock).await
}

pub(super) async fn probe_once(
    inner: &ProxyShared,
    target: &str,
    key: &str,
    headers: &Upstream,
    lock: &tokio::sync::Mutex<()>,
) -> Result<Option<u64>, Failure> {
    let _turn = lock.lock().await;
    if inner.ignores_range(key) {
        return Ok(None);
    }
    if let Some(total) = inner.known_total(key) {
        return Ok(Some(total));
    }
    let probe = probe_total(inner, target, headers.clone()).await?;
    if probe.ignored_range {
        inner.mark_ignores_range(key);
        return Ok(None);
    }
    Ok(probe.total.inspect(|total| inner.remember_total(key, *total)))
}

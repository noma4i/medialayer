use crate::proxy::{Priority, Proxy};
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LoadError(pub String);

#[async_trait]
pub trait Loader: Send + Sync {
    fn cache_key(&self, url: &str, headers: &HashMap<String, String>) -> String;
    // Notify progress only after writing bytes or completing a stored block.
    async fn load(&self, url: &str, headers: &HashMap<String, String>, priority: Priority, progress: &Notify) -> Result<u64, LoadError>;
    async fn load_first_block(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<u64, LoadError>;
    async fn load_all_blocks(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
        progress: &Notify,
    ) -> Result<u64, LoadError>;
    async fn keep(&self, _url: &str, _headers: &HashMap<String, String>) {}
    async fn playlist(&self, url: &str, headers: &HashMap<String, String>) -> Option<std::sync::Arc<crate::proxy::StoredPlaylist>>;
    async fn stored_len(&self, url: &str, headers: &HashMap<String, String>) -> Option<u64>;
    async fn evict(&self, url: &str, headers: &HashMap<String, String>);
}

pub struct ProxyLoader {
    proxy: Proxy,
}

impl ProxyLoader {
    pub fn new(proxy: Proxy) -> ProxyLoader {
        ProxyLoader { proxy }
    }
}

#[async_trait]
impl Loader for ProxyLoader {
    fn cache_key(&self, url: &str, headers: &HashMap<String, String>) -> String {
        self.proxy.key_for(url, headers)
    }

    async fn load(&self, url: &str, headers: &HashMap<String, String>, priority: Priority, progress: &Notify) -> Result<u64, LoadError> {
        self.proxy.fetch_into_store(url, headers, priority, Some(progress)).await.map_err(|error| LoadError(error.to_string()))?;
        Ok(self.proxy.stored_len(url, headers).await.unwrap_or(0))
    }

    async fn load_first_block(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<u64, LoadError> {
        self.proxy.fetch_first_block(url, headers, priority).await.map_err(|error| LoadError(error.to_string()))?;
        Ok(0)
    }

    async fn load_all_blocks(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
        progress: &Notify,
    ) -> Result<u64, LoadError> {
        self.proxy.fetch_all_blocks(url, headers, priority, progress).await.map_err(|error| LoadError(error.to_string()))
    }

    async fn keep(&self, url: &str, headers: &HashMap<String, String>) {
        self.proxy.keep(url, headers).await;
    }

    async fn playlist(&self, url: &str, headers: &HashMap<String, String>) -> Option<std::sync::Arc<crate::proxy::StoredPlaylist>> {
        self.proxy.playlist_plan(url, headers).await
    }

    async fn stored_len(&self, url: &str, headers: &HashMap<String, String>) -> Option<u64> {
        self.proxy.stored_len(url, headers).await
    }

    async fn evict(&self, url: &str, headers: &HashMap<String, String>) {
        self.proxy.remove_stored(url, headers).await;
    }
}

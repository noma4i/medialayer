use crate::identity::cache_key;
use crate::proxy::{ByteStore, Priority, Proxy};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

const MAX_PLAYLIST_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LoadError(pub String);

#[async_trait]
pub trait Loader: Send + Sync {
    async fn load(&self, url: &str, headers: &HashMap<String, String>, priority: Priority) -> Result<(), LoadError>;
    async fn load_first_block(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), LoadError>;
    async fn stored(&self, url: &str) -> Option<Vec<u8>>;
}

pub struct ProxyLoader {
    proxy: Proxy,
    store: Arc<dyn ByteStore>,
}

impl ProxyLoader {
    pub fn new(proxy: Proxy, store: Arc<dyn ByteStore>) -> ProxyLoader {
        ProxyLoader { proxy, store }
    }
}

#[async_trait]
impl Loader for ProxyLoader {
    async fn load(&self, url: &str, headers: &HashMap<String, String>, priority: Priority) -> Result<(), LoadError> {
        self.proxy.fetch_into_store(url, headers, priority).await.map_err(|error| LoadError(error.to_string()))
    }

    async fn load_first_block(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), LoadError> {
        self.proxy.fetch_first_block(url, headers, priority).await.map_err(|error| LoadError(error.to_string()))
    }

    async fn stored(&self, url: &str) -> Option<Vec<u8>> {
        let key = cache_key(url);
        let length = self.store.len(&key).await.filter(|length| *length <= MAX_PLAYLIST_BYTES)?;
        self.store.get_range(&key, 0, length).await
    }
}

use crate::memory::InputLimits;
use crate::proxy::{Priority, Proxy};
use async_trait::async_trait;
use std::collections::HashMap;

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
    async fn stored(&self, url: &str, headers: &HashMap<String, String>) -> Option<Vec<u8>>;
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

    async fn stored(&self, url: &str, headers: &HashMap<String, String>) -> Option<Vec<u8>> {
        self.proxy.read_stored(url, headers, InputLimits::default().playlist_bytes).await
    }
}

use async_trait::async_trait;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub content_type: Option<String>,
    pub total_length: Option<u64>,
}

#[async_trait]
pub trait ByteStore: Send + Sync {
    async fn len(&self, key: &str) -> Option<u64>;
    async fn contains(&self, key: &str) -> bool;
    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>>;
    async fn meta(&self, key: &str) -> Option<Meta>;
    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>>;
    async fn remove(&self, key: &str);
    async fn keep(&self, _key: &str) {}
}

#[async_trait]
pub trait StoreWriter: Send {
    // Dropping a writer without commit discards the entry, like abort.
    async fn append(&mut self, bytes: &[u8]) -> bool;
    async fn commit(self: Box<Self>) -> bool;
    // Commits only when exactly `length` bytes were appended; a short or long body is discarded.
    async fn commit_exact(self: Box<Self>, length: u64) -> bool;
    async fn abort(self: Box<Self>);
}

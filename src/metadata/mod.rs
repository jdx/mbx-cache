mod memory;
mod postgres;

use crate::model::{ActionResult, Digest};
use async_trait::async_trait;

pub use memory::MemoryMetadata;
pub use postgres::PostgresMetadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Created,
    AlreadyExists,
    Conflict,
}

#[async_trait]
pub trait MetadataStore: Send + Sync {
    async fn blob_visible(&self, namespace: &str, digest: &Digest) -> anyhow::Result<bool>;
    async fn register_blob(&self, namespace: &str, digest: &Digest) -> anyhow::Result<()>;
    async fn get(&self, namespace: &str, action: &Digest) -> anyhow::Result<Option<ActionResult>>;
    async fn commit(
        &self,
        namespace: &str,
        action: &Digest,
        result: &ActionResult,
    ) -> anyhow::Result<CommitOutcome>;
}

pub async fn from_url(url: &str) -> anyhow::Result<std::sync::Arc<dyn MetadataStore>> {
    if url == "memory://" {
        Ok(std::sync::Arc::new(MemoryMetadata::default()))
    } else {
        Ok(std::sync::Arc::new(PostgresMetadata::connect(url).await?))
    }
}

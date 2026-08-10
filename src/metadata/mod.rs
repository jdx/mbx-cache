mod memory;
mod postgres;

use crate::model::{ActionResult, Digest, TaskActionManifest};
use async_trait::async_trait;

pub use memory::MemoryMetadata;
pub use postgres::PostgresMetadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Created,
    AlreadyExists,
    Conflict,
}

pub struct ManifestRecord {
    pub etag: String,
    pub manifest: TaskActionManifest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestCommitOutcome {
    Created,
    Updated,
    PreconditionFailed,
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
    async fn get_manifest(
        &self,
        namespace: &str,
        key: &Digest,
    ) -> anyhow::Result<Option<ManifestRecord>>;
    async fn commit_manifest(
        &self,
        namespace: &str,
        key: &Digest,
        expected_etag: Option<&str>,
        etag: &str,
        manifest: &TaskActionManifest,
    ) -> anyhow::Result<ManifestCommitOutcome>;
}

pub async fn from_url(url: &str) -> anyhow::Result<std::sync::Arc<dyn MetadataStore>> {
    if url == "memory://" {
        Ok(std::sync::Arc::new(MemoryMetadata::default()))
    } else {
        Ok(std::sync::Arc::new(PostgresMetadata::connect(url).await?))
    }
}

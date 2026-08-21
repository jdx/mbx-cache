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
    async fn visible_blobs(
        &self,
        namespace: &str,
        digests: &[Digest],
    ) -> anyhow::Result<Vec<Digest>>;
    async fn blob_visible(&self, namespace: &str, digest: &Digest) -> anyhow::Result<bool> {
        Ok(!self
            .visible_blobs(namespace, std::slice::from_ref(digest))
            .await?
            .is_empty())
    }
    async fn register_blob(&self, namespace: &str, digest: &Digest) -> anyhow::Result<()>;
    /// Record that these blobs were served to a client.
    ///
    /// `namespace_blobs.last_accessed_at` otherwise only ever holds the time a
    /// blob was uploaded, which would make a future garbage collector evict the
    /// blobs a build depends on most. Recording an access is best-effort and
    /// must never fail a read; stores without durable metadata do nothing.
    async fn touch_blobs(&self, _namespace: &str, _digests: &[Digest]) -> anyhow::Result<()> {
        Ok(())
    }
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

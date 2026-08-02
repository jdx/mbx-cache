use async_trait::async_trait;
use sqlx::{PgPool, Row};

use super::{CommitOutcome, MetadataStore};
use crate::model::{ActionResultEnvelope, Digest};

pub struct PostgresMetadata {
    pool: PgPool,
}

impl PostgresMetadata {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPool::connect(url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl MetadataStore for PostgresMetadata {
    async fn blob_visible(&self, namespace: &str, digest: &Digest) -> anyhow::Result<bool> {
        Ok(sqlx::query("SELECT 1 FROM namespace_blobs WHERE namespace = $1 AND algorithm = $2 AND hash = $3 AND size = $4")
            .bind(namespace).bind(digest.algorithm.to_string()).bind(&digest.hash).bind(digest.size as i64)
            .fetch_optional(&self.pool).await?.is_some())
    }

    async fn register_blob(&self, namespace: &str, digest: &Digest) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO namespace_blobs (namespace, algorithm, hash, size) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(namespace).bind(digest.algorithm.to_string()).bind(&digest.hash).bind(digest.size as i64)
            .execute(&self.pool).await?;
        Ok(())
    }

    async fn get(
        &self,
        namespace: &str,
        action: &Digest,
    ) -> anyhow::Result<Option<ActionResultEnvelope>> {
        let row = sqlx::query("SELECT result FROM action_results WHERE namespace = $1 AND algorithm = $2 AND hash = $3 AND size = $4")
            .bind(namespace).bind(action.algorithm.to_string()).bind(&action.hash).bind(action.size as i64)
            .fetch_optional(&self.pool).await?;
        row.map(|row| serde_json::from_value(row.get("result")).map_err(Into::into))
            .transpose()
    }

    async fn commit(
        &self,
        namespace: &str,
        action: &Digest,
        result: &ActionResultEnvelope,
    ) -> anyhow::Result<CommitOutcome> {
        let encoded = serde_json::to_value(result)?;
        let inserted = sqlx::query("INSERT INTO action_results (namespace, algorithm, hash, size, result) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
            .bind(namespace).bind(action.algorithm.to_string()).bind(&action.hash).bind(action.size as i64).bind(&encoded)
            .execute(&self.pool).await?.rows_affected();
        if inserted == 1 {
            return Ok(CommitOutcome::Created);
        }
        let existing = self
            .get(namespace, action)
            .await?
            .ok_or_else(|| anyhow::anyhow!("action result disappeared after conflict"))?;
        if serde_json::to_value(existing)? == encoded {
            Ok(CommitOutcome::AlreadyExists)
        } else {
            Ok(CommitOutcome::Conflict)
        }
    }
}

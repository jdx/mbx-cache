use async_trait::async_trait;
use sqlx::{PgPool, Row};

use super::{CommitOutcome, ManifestCommitOutcome, ManifestRecord, MetadataStore};
use crate::model::{ActionResult, Digest, TaskActionManifest};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// How stale a blob's recorded access has to be before a read refreshes it.
const ACCESS_REFRESH_INTERVAL: &str = "1 hour";

pub struct PostgresMetadata {
    pool: PgPool,
}

impl PostgresMetadata {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPool::connect(url).await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }
}

fn representable_digests(digests: &[Digest]) -> Vec<(&Digest, i64)> {
    digests
        .iter()
        .filter_map(|digest| i64::try_from(digest.size).ok().map(|size| (digest, size)))
        .collect()
}

#[async_trait]
impl MetadataStore for PostgresMetadata {
    async fn visible_blobs(
        &self,
        namespace: &str,
        digests: &[Digest],
    ) -> anyhow::Result<Vec<Digest>> {
        let digests = representable_digests(digests);
        if digests.is_empty() {
            return Ok(Vec::new());
        }
        let algorithms = digests
            .iter()
            .map(|(digest, _)| digest.algorithm.to_string())
            .collect::<Vec<_>>();
        let hashes = digests
            .iter()
            .map(|(digest, _)| digest.hash.clone())
            .collect::<Vec<_>>();
        let sizes = digests.iter().map(|(_, size)| *size).collect::<Vec<_>>();
        let rows = sqlx::query(
            "SELECT requested.ordinality \
             FROM UNNEST($2::text[], $3::text[], $4::bigint[]) WITH ORDINALITY \
                  AS requested(algorithm, hash, size, ordinality) \
             JOIN namespace_blobs AS blobs \
               ON blobs.algorithm = requested.algorithm \
              AND blobs.hash = requested.hash \
              AND blobs.size = requested.size \
             WHERE blobs.namespace = $1 \
             ORDER BY requested.ordinality",
        )
        .bind(namespace)
        .bind(algorithms)
        .bind(hashes)
        .bind(sizes)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let ordinal: i64 = row.try_get("ordinality")?;
                let index = usize::try_from(ordinal - 1)?;
                digests
                    .get(index)
                    .map(|(digest, _)| (*digest).clone())
                    .ok_or_else(|| anyhow::anyhow!("database returned an invalid blob ordinal"))
            })
            .collect()
    }

    async fn touch_blobs(&self, namespace: &str, digests: &[Digest]) -> anyhow::Result<()> {
        let digests = representable_digests(digests);
        if digests.is_empty() {
            return Ok(());
        }
        let algorithms = digests
            .iter()
            .map(|(digest, _)| digest.algorithm.to_string())
            .collect::<Vec<_>>();
        let hashes = digests
            .iter()
            .map(|(digest, _)| digest.hash.clone())
            .collect::<Vec<_>>();
        let sizes = digests.iter().map(|(_, size)| *size).collect::<Vec<_>>();
        // Skip blobs touched recently so a frequently served blob costs at most
        // one write per interval rather than one per read.
        sqlx::query(
            "UPDATE namespace_blobs AS blobs \
             SET last_accessed_at = now() \
             FROM UNNEST($2::text[], $3::text[], $4::bigint[]) \
                  AS requested(algorithm, hash, size) \
             WHERE blobs.namespace = $1 \
               AND blobs.algorithm = requested.algorithm \
               AND blobs.hash = requested.hash \
               AND blobs.size = requested.size \
               AND blobs.last_accessed_at < now() - $5::interval",
        )
        .bind(namespace)
        .bind(algorithms)
        .bind(hashes)
        .bind(sizes)
        .bind(ACCESS_REFRESH_INTERVAL)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn register_blob(&self, namespace: &str, digest: &Digest) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO namespace_blobs (namespace, algorithm, hash, size) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(namespace).bind(digest.algorithm.to_string()).bind(&digest.hash).bind(digest.size as i64)
            .execute(&self.pool).await?;
        Ok(())
    }

    async fn get(&self, namespace: &str, action: &Digest) -> anyhow::Result<Option<ActionResult>> {
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
        result: &ActionResult,
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

    async fn get_manifest(
        &self,
        namespace: &str,
        key: &Digest,
    ) -> anyhow::Result<Option<ManifestRecord>> {
        let row = sqlx::query("SELECT etag, manifest FROM action_manifests WHERE namespace = $1 AND algorithm = $2 AND hash = $3 AND size = $4")
            .bind(namespace).bind(key.algorithm.to_string()).bind(&key.hash).bind(key.size as i64)
            .fetch_optional(&self.pool).await?;
        row.map(|row| {
            Ok(ManifestRecord {
                etag: row.get("etag"),
                manifest: serde_json::from_value(row.get("manifest"))?,
            })
        })
        .transpose()
    }

    async fn commit_manifest(
        &self,
        namespace: &str,
        key: &Digest,
        expected_etag: Option<&str>,
        etag: &str,
        manifest: &TaskActionManifest,
    ) -> anyhow::Result<ManifestCommitOutcome> {
        let manifest = serde_json::to_value(manifest)?;
        let rows = if let Some(expected_etag) = expected_etag {
            sqlx::query("UPDATE action_manifests SET etag = $5, manifest = $6, updated_at = now() WHERE namespace = $1 AND algorithm = $2 AND hash = $3 AND size = $4 AND etag = $7")
                .bind(namespace).bind(key.algorithm.to_string()).bind(&key.hash).bind(key.size as i64)
                .bind(etag).bind(&manifest).bind(expected_etag)
                .execute(&self.pool).await?.rows_affected()
        } else {
            sqlx::query("INSERT INTO action_manifests (namespace, algorithm, hash, size, etag, manifest) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING")
                .bind(namespace).bind(key.algorithm.to_string()).bind(&key.hash).bind(key.size as i64)
                .bind(etag).bind(&manifest)
                .execute(&self.pool).await?.rows_affected()
        };
        Ok(if rows == 0 {
            ManifestCommitOutcome::PreconditionFailed
        } else if expected_etag.is_some() {
            ManifestCommitOutcome::Updated
        } else {
            ManifestCommitOutcome::Created
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{MIGRATOR, representable_digests};
    use crate::model::{Algorithm, Digest};

    #[test]
    fn embedded_migration_versions_are_unique() {
        let mut versions = BTreeSet::new();
        for migration in MIGRATOR.iter() {
            assert!(
                versions.insert(migration.version),
                "migration version {} is duplicated",
                migration.version
            );
        }
    }

    #[test]
    fn unrepresentable_blob_sizes_are_not_queried() {
        let representable = Digest {
            algorithm: Algorithm::Blake3,
            hash: "0".repeat(64),
            size: i64::MAX as u64,
        };
        let unrepresentable = Digest {
            algorithm: Algorithm::Blake3,
            hash: "1".repeat(64),
            size: i64::MAX as u64 + 1,
        };

        assert_eq!(
            representable_digests(&[representable.clone(), unrepresentable]),
            vec![(&representable, i64::MAX)]
        );
    }
}

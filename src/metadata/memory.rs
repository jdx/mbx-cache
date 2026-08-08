use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet},
    sync::RwLock,
};

use super::{CommitOutcome, MetadataStore};
use crate::model::{ActionResult, Digest};

#[derive(Default)]
pub struct MemoryMetadata {
    entries: RwLock<HashMap<(String, Digest), Vec<u8>>>,
    blobs: RwLock<HashSet<(String, Digest)>>,
}

#[async_trait]
impl MetadataStore for MemoryMetadata {
    async fn blob_visible(&self, namespace: &str, digest: &Digest) -> anyhow::Result<bool> {
        Ok(self
            .blobs
            .read()
            .expect("metadata lock poisoned")
            .contains(&(namespace.to_owned(), digest.clone())))
    }

    async fn register_blob(&self, namespace: &str, digest: &Digest) -> anyhow::Result<()> {
        self.blobs
            .write()
            .expect("metadata lock poisoned")
            .insert((namespace.to_owned(), digest.clone()));
        Ok(())
    }

    async fn get(&self, namespace: &str, action: &Digest) -> anyhow::Result<Option<ActionResult>> {
        let entries = self.entries.read().expect("metadata lock poisoned");
        entries
            .get(&(namespace.to_owned(), action.clone()))
            .map(|value| serde_json::from_slice(value).map_err(Into::into))
            .transpose()
    }

    async fn commit(
        &self,
        namespace: &str,
        action: &Digest,
        result: &ActionResult,
    ) -> anyhow::Result<CommitOutcome> {
        let encoded = serde_json::to_vec(result)?;
        let mut entries = self.entries.write().expect("metadata lock poisoned");
        match entries.get(&(namespace.to_owned(), action.clone())) {
            None => {
                entries.insert((namespace.to_owned(), action.clone()), encoded);
                Ok(CommitOutcome::Created)
            }
            Some(existing) if *existing == encoded => Ok(CommitOutcome::AlreadyExists),
            Some(_) => Ok(CommitOutcome::Conflict),
        }
    }
}

//! Project rules are inputs to derived relationships, independently of source hashes.

use super::{Pipeline, PipelineError, PipelineResult};
use crate::project_resolver::{
    persist::ResolutionPersistence,
    provider::ProjectResolutionProvider,
    providers::{javascript::JavaScriptProvider, typescript::TypeScriptProvider},
};
use crate::storage::{DocumentIndex, MetadataKey};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

impl Pipeline {
    pub(super) fn complete_project_bindings(
        &self,
        index: &DocumentIndex,
        fingerprint: Option<u64>,
    ) -> PipelineResult<()> {
        if let Some(fingerprint) = fingerprint
            && index.query_metadata(MetadataKey::ResolutionFingerprint)? != Some(fingerprint)
        {
            index.start_batch()?;
            if let Err(error) =
                index.store_metadata(MetadataKey::ResolutionFingerprint, fingerprint)
            {
                let _ = index.rollback_batch();
                return Err(error.into());
            }
            index.commit_batch()?;
        }
        Ok(())
    }

    pub(super) fn refresh_project_bindings(
        &self,
        index: &DocumentIndex,
    ) -> PipelineResult<Option<u64>> {
        let previous = index.query_metadata(MetadataKey::ResolutionFingerprint)?;
        let configured = ["typescript", "javascript"].iter().any(|language| {
            self.settings
                .languages
                .get(*language)
                .is_some_and(|config| !config.config_files.is_empty())
        });
        if !configured && previous.is_none() {
            return Ok(None);
        }
        let providers: [&dyn ProjectResolutionProvider; 2] =
            [&TypeScriptProvider::new(), &JavaScriptProvider::new()];
        let persistence = ResolutionPersistence::new(&self.settings.resolution_dir());
        let mut digest = Sha256::new();
        for provider in providers {
            provider.rebuild_cache(&self.settings).map_err(|error| {
                PipelineError::Index(crate::IndexError::General(format!(
                    "Cannot refresh project bindings: {error}"
                )))
            })?;
            let rules = persistence.load(provider.language_id()).map_err(|error| {
                PipelineError::Index(crate::IndexError::General(format!(
                    "Cannot read project bindings: {error}"
                )))
            })?;
            // HashMap iteration is unstable. JSON Value uses ordered object keys.
            let canonical = serde_json::to_value(&rules).map_err(|error| {
                PipelineError::Index(crate::IndexError::General(error.to_string()))
            })?;
            digest.update(provider.language_id().as_bytes());
            digest.update(canonical.to_string().as_bytes());
        }
        let hash = digest.finalize();
        let fingerprint = u64::from_le_bytes(hash[..8].try_into().expect("SHA256 has eight bytes"));
        if previous != Some(fingerprint) {
            let paths: Vec<PathBuf> = index
                .get_all_indexed_paths()?
                .into_iter()
                .filter(|path| {
                    matches!(
                        path.extension().and_then(|ext| ext.to_str()),
                        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts")
                    )
                })
                .collect();
            super::dependencies::invalidate_importers(index, &paths)?;
        }
        Ok(Some(fingerprint))
    }
}

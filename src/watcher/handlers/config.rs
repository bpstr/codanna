//! Handler for configuration file changes.
//!
//! Loading proposes a complete snapshot; the watcher validates and acknowledges
//! it only after publication, so failed changes cannot advance the diff baseline.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::config::Settings;
use crate::watcher::{WatchAction, WatchError, WatchHandler};

pub struct ConfigFileHandler {
    settings_path: PathBuf,
}

impl ConfigFileHandler {
    /// Watch a settings file after checking that its initial snapshot loads.
    pub fn new(settings_path: PathBuf) -> Result<Self, WatchError> {
        Settings::load_from(&settings_path).map_err(|error| WatchError::ConfigError {
            reason: format!("Failed to load config: {error}"),
        })?;
        Ok(Self { settings_path })
    }
}

#[async_trait]
impl WatchHandler for ConfigFileHandler {
    fn name(&self) -> &str {
        "config"
    }

    fn reloads_config(&self) -> bool {
        true
    }

    fn matches(&self, path: &Path) -> bool {
        crate::documents::store::normalize_source_path(path)
            == crate::documents::store::normalize_source_path(&self.settings_path)
    }

    async fn tracked_paths(&self) -> Vec<PathBuf> {
        vec![self.settings_path.clone()]
    }

    async fn on_modify(&self, _path: &Path) -> Result<WatchAction, WatchError> {
        let path = self.settings_path.clone();
        let settings = crate::runtime::blocking(move || {
            let mut settings = Settings::load_from(&path)?;
            settings.index_path = crate::init::resolve_index_path(&settings, Some(&path));
            Ok::<_, Box<figment::Error>>(settings)
        })
        .await
        .map_err(|error| WatchError::ConfigError {
            reason: error.to_string(),
        })?
        .map_err(|error| WatchError::ConfigError {
            reason: format!("Failed to reload config: {error}"),
        })?;
        Ok(WatchAction::ReloadSettings {
            settings: Box::new(settings),
        })
    }

    async fn on_delete(&self, _path: &Path) -> Result<WatchAction, WatchError> {
        Err(WatchError::ConfigError {
            reason: format!(
                "Config file {} was deleted; retaining the running settings",
                self.settings_path.display()
            ),
        })
    }
}

from pathlib import Path

facade = Path("src/indexing/facade.rs")
s = facade.read_text()
marker = '''    /// Get the index base path.
    pub fn index_base(&self) -> &Path {
'''
method = '''    /// Replace runtime settings and rebuild the mutation pipeline.
    ///
    /// The storage identity is immutable for a live facade: changing workspace
    /// root or index path requires a process restart so query and mutation
    /// components cannot point at different generations.
    pub fn reload_settings(&mut self, settings: Settings) -> Result<(), String> {
        let new_index_base = if let Some(ref workspace_root) = settings.workspace_root {
            workspace_root.join(&settings.index_path)
        } else {
            settings.index_path.clone()
        };
        if new_index_base != self.index_base {
            return Err(format!(
                "live config reload cannot change index storage from {} to {}; restart codanna",
                self.index_base.display(),
                new_index_base.display()
            ));
        }

        let settings = Arc::new(settings);
        self.pipeline = Pipeline::with_settings(Arc::clone(&settings));
        self.settings = settings;
        Ok(())
    }

'''
if marker not in s:
    raise SystemExit("facade insertion marker not found")
facade.write_text(s.replace(marker, method + marker, 1))

watcher = Path("src/watcher/unified.rs")
s = watcher.read_text()
marker = '''            WatchAction::ReloadConfig { added, removed } => {
                if !added.is_empty() {
'''
replacement = '''            WatchAction::ReloadConfig { added, removed } => {
                // Apply the new settings before indexing or refreshing handlers.
                // Otherwise CodeFileHandler continues deriving eligibility from
                // stale facade settings and newly-added roots stay unwatched.
                let settings_path = self
                    .workspace_root
                    .join(crate::init::local_dir_name())
                    .join("settings.toml");
                match crate::config::Settings::load_from(&settings_path) {
                    Ok(mut settings) => {
                        if settings.workspace_root.is_none() {
                            settings.workspace_root = Some(self.workspace_root.clone());
                        }
                        let mut indexer = self.facade.write().await;
                        if let Err(e) = indexer.reload_settings(settings) {
                            tracing::error!("[config] failed to apply reloaded settings: {e}");
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        tracing::error!("[config] failed to reload settings: {e}");
                        return Ok(());
                    }
                }

                if !added.is_empty() {
'''
if marker not in s:
    raise SystemExit("ReloadConfig marker not found")
watcher.write_text(s.replace(marker, replacement, 1))

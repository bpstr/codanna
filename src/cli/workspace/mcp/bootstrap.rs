//! Bounded, code-only first indexing. Build privately and publish only a complete
//! generation. Existing nonempty indexes are never cleared or silently rebuilt.
mod discovery;

use super::budget::Budget;
use crate::init::workspaces::{Workspace, confined_index_path, read_settings};
use crate::storage::IndexMetadata;
use rmcp::model::ErrorData;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const MAX_FILES: u32 = 50_000;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) enum Status {
    Ready,
    Empty,
    Busy,
}
struct Build {
    _lock: File,
    staging: tempfile::TempDir,
    config: PathBuf,
    destination: PathBuf,
    root: PathBuf,
    source_config: PathBuf,
    source_config_bytes: Vec<u8>,
}
enum Plan {
    Complete(Status),
    Build(Build),
}

pub(super) async fn ensure(
    workspace: Workspace,
    budget: Budget,
    slots: Arc<Semaphore>,
    executable: PathBuf,
    ct: CancellationToken,
) -> Result<Status, ErrorData> {
    let _permit = tokio::select! {
        _ = ct.cancelled() => return Err(super::internal("Workspace bootstrap cancelled")),
        result = slots.acquire_owned() => result.map_err(super::internal)?,
    };
    let selected = workspace.clone();
    let plan = budget
        .run(&ct, move |token| plan(&selected, &token))
        .await?;
    let build = match plan {
        Plan::Complete(status) => return Ok(status),
        Plan::Build(build) => build,
    };
    let mut command = tokio::process::Command::new(executable);
    command
        .current_dir(&build.root)
        .arg("--config")
        .arg(&build.config)
        .args(["index", "--threads", "2", "--max-files"])
        // One overflow witness distinguishes a complete allowed generation from
        // a truncated pass if files arrive after the discovery preflight.
        .arg((MAX_FILES + 1).to_string())
        .arg("--no-progress")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Remove overlays that could redirect storage or re-enable remote inference.
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("CI_")
        {
            command.env_remove(key);
        }
    }
    command
        .env_remove("CODANNA_RECALL_INDEX")
        .env_remove("CODANNA_RECALL_WORKSPACE");
    let mut child = command.spawn().map_err(super::internal)?;
    let status = tokio::select! {
        _ = ct.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(300), child.wait()) => Some(result),
    };
    let successful = match status {
        Some(Ok(Ok(status))) => status.success(),
        _ => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            false
        }
    };
    if !successful {
        return Err(super::internal(
            "Initial code indexing failed or exceeded its five-minute budget. No existing index was replaced. Run codanna index in the workspace for detailed recovery output.",
        ));
    }
    budget.run(&ct, move |token| publish(build, &token)).await
}

fn read_config_bytes(path: &Path) -> Result<Vec<u8>, ErrorData> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(super::internal)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(super::internal)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(super::internal("Workspace configuration exceeds 1 MiB"));
    }
    Ok(bytes)
}

fn validate_generation(metadata: &IndexMetadata) -> Result<Status, ErrorData> {
    if metadata.emission_version != Some(crate::storage::metadata::EMISSION_SEMANTICS_VERSION) {
        return Err(super::internal(
            "Initial index has incompatible emission semantics",
        ));
    }
    if metadata.file_count > MAX_FILES {
        return Err(super::internal(
            "Source files exceeded the automatic indexing limit during the build. The partial generation was not published; use explicit codanna index.",
        ));
    }
    // README/config-only directories and disabled languages can produce a valid
    // index with no indexed files. Do not publish that as permanently ready: the
    // session must still bootstrap when its first supported source file arrives.
    // A file with zero symbols is different and remains a valid indexed file.
    if metadata.file_count == 0 {
        Ok(Status::Empty)
    } else {
        Ok(Status::Ready)
    }
}

fn publish(build: Build, ct: &CancellationToken) -> Result<Status, ErrorData> {
    if ct.is_cancelled() {
        return Err(super::internal(
            "Workspace bootstrap cancelled before publication",
        ));
    }
    if read_config_bytes(&build.source_config)? != build.source_config_bytes {
        return Err(super::internal(
            "Workspace configuration changed during initial indexing. No generation was published; retry with the current configuration.",
        ));
    }
    let staged = build.staging.path().join("index");
    let mut metadata = IndexMetadata::load(&staged).map_err(super::internal)?;
    let status = validate_generation(&metadata)?;
    if matches!(status, Status::Empty) {
        return Ok(status);
    }
    if !staged.join("tantivy/meta.json").is_file() {
        return Err(super::internal(
            "Initial index has no committed code generation",
        ));
    }
    // A concurrent explicit writer wins. Never replace its nonempty output.
    if build.destination.exists()
        && !crate::cli::automatic::is_uninitialized_index(&build.destination)
            .map_err(super::internal)?
    {
        IndexMetadata::load(&build.destination).map_err(super::internal)?;
        return Ok(Status::Ready);
    }
    if build.destination.exists() {
        // These operations refuse nonempty directories, including a writer's
        // newly added files. Do not use recursive removal for live storage.
        if build.destination.join("tantivy").is_dir() {
            fs::remove_dir(build.destination.join("tantivy")).map_err(super::internal)?;
        }
        fs::remove_dir(&build.destination).map_err(super::internal)?;
    }
    if let crate::storage::DataSource::Tantivy { path, .. } = &mut metadata.data_source {
        *path = build.destination.join("tantivy");
    }
    metadata.save(&staged).map_err(super::internal)?;
    fs::rename(&staged, &build.destination).map_err(super::internal)?;
    #[cfg(unix)]
    File::open(
        build
            .destination
            .parent()
            .ok_or_else(|| super::internal("Index has no parent"))?,
    )
    .and_then(|file| file.sync_all())
    .map_err(super::internal)?;
    Ok(Status::Ready)
}

fn try_bootstrap_lock(lock: &File) -> Result<bool, ErrorData> {
    // Use the repository's existing fs4 dependency, not File::try_lock (1.89+).
    // fs4 0.13 returns Ok(false) for contention; it is not lock ownership.
    fs4::fs_std::FileExt::try_lock_exclusive(lock).map_err(super::internal)
}

fn plan(workspace: &Workspace, ct: &CancellationToken) -> Result<Plan, ErrorData> {
    let source_config_bytes = read_config_bytes(&workspace.config_path)?;
    let mut settings = read_settings(&workspace.root).map_err(super::internal)?;
    if read_config_bytes(&workspace.config_path)? != source_config_bytes {
        return Err(super::internal(
            "Workspace configuration changed during setup; retry",
        ));
    }
    let destination = confined_index_path(&workspace.root, &settings).map_err(super::internal)?;
    if destination.join("index.meta").is_file() {
        IndexMetadata::load(&destination).map_err(super::internal)?;
        return Ok(Plan::Complete(Status::Ready));
    }
    if !crate::cli::automatic::is_uninitialized_index(&destination).map_err(super::internal)? {
        return Err(super::internal(
            "Existing index is incomplete. Automatic setup will not replace it; run codanna index for recovery.",
        ));
    }
    let state = workspace.root.join(crate::init::local_dir_name());
    let lock_path = state.join("bootstrap.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.is_symlink()) {
        return Err(super::internal("Bootstrap lock must not be a symlink"));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(super::internal)?;
    if !try_bootstrap_lock(&lock)? {
        return Ok(Plan::Complete(Status::Busy));
    }
    if destination.join("index.meta").is_file() {
        return Ok(Plan::Complete(Status::Ready));
    }
    let mut roots = Vec::new();
    for source in &settings.indexing.indexed_paths {
        let path = workspace
            .root
            .join(source)
            .canonicalize()
            .map_err(super::internal)?;
        if !path.starts_with(&workspace.root) {
            return Err(super::internal(
                "Automatic indexing cannot include external source roots",
            ));
        }
        roots.push(path);
    }
    if roots.is_empty() {
        roots.push(workspace.root.clone());
    }
    if !discovery::has_sources(&settings, &roots, ct)? {
        return Ok(Plan::Complete(Status::Empty));
    }
    let staging = tempfile::Builder::new()
        .prefix(".bootstrap-")
        .tempdir_in(&state)
        .map_err(super::internal)?;
    settings.workspace_root = Some(workspace.root.clone());
    settings.index_path = staging.path().join("index");
    settings.indexing.indexed_paths = roots;
    settings.indexing.parallelism = 2;
    settings.semantic_search.enabled = false;
    settings.file_watch.enabled = false;
    let config = staging.path().join("settings.toml");
    fs::write(
        &config,
        toml::to_string_pretty(&settings).map_err(super::internal)?,
    )
    .map_err(super::internal)?;
    Ok(Plan::Build(Build {
        _lock: lock,
        staging,
        config,
        destination,
        root: workspace.root.clone(),
        source_config: workspace.config_path.clone(),
        source_config_bytes,
    }))
}

#[cfg(test)]
mod tests;

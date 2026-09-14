//! Bounded, code-only first indexing. Build privately and publish only a complete
//! generation. Existing nonempty indexes are never cleared or silently rebuilt.
use super::budget::Budget;
use crate::init::workspaces::{Workspace, confined_index_path, read_settings};
use crate::storage::IndexMetadata;
use rmcp::model::ErrorData;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub(super) enum Status { Ready, Empty, Busy }
struct Build {
    _lock: File,
    staging: tempfile::TempDir,
    config: PathBuf,
    destination: PathBuf,
    root: PathBuf,
}
enum Plan { Complete(Status), Build(Build) }

pub(super) async fn ensure(workspace: Workspace, budget: Budget, slots: Arc<Semaphore>, executable: PathBuf, ct: CancellationToken) -> Result<Status, ErrorData> {
    let _permit = tokio::select! {
        _ = ct.cancelled() => return Err(super::internal("Workspace bootstrap cancelled")),
        result = slots.acquire_owned() => result.map_err(super::internal)?,
    };
    let selected = workspace.clone();
    let plan = budget.run(&ct, move |token| plan(&selected, &token)).await?;
    let Plan::Build(build) = plan else { if let Plan::Complete(status) = plan { return Ok(status); } unreachable!() };
    let mut command = tokio::process::Command::new(executable);
    command.current_dir(&build.root).args(["--config"]).arg(&build.config).args(["index", "--threads", "2", "--max-files", "50000", "--no-progress"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    // Remove overlays that could redirect storage or re-enable remote inference.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_ascii_uppercase().starts_with("CI_") { command.env_remove(key); }
    }
    command.env_remove("CODANNA_RECALL_INDEX").env_remove("CODANNA_RECALL_WORKSPACE");
    let mut child = command.spawn().map_err(super::internal)?;
    let status = tokio::select! {
        _ = ct.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(300), child.wait()) => Some(result),
    };
    let successful = match status {
        Some(Ok(Ok(status))) => status.success(),
        _ => { let _ = child.start_kill(); let _ = child.wait().await; false }
    };
    if !successful { return Err(super::internal("Initial code indexing failed or exceeded its five-minute budget. No existing index was replaced. Run codanna index in the workspace for detailed recovery output.")); }
    budget.run(&ct, move |token| {
        if token.is_cancelled() { return Err(super::internal("Workspace bootstrap cancelled before publication")); }
        let staged = build.staging.path().join("index");
        let mut metadata = IndexMetadata::load(&staged).map_err(super::internal)?;
        if metadata.emission_version != Some(crate::storage::metadata::EMISSION_SEMANTICS_VERSION) || !staged.join("tantivy/meta.json").is_file() {
            return Err(super::internal("Initial index did not produce a complete compatible generation"));
        }
        // A concurrent explicit writer wins. Never replace its nonempty output.
        if build.destination.exists() && !crate::cli::automatic::is_uninitialized_index(&build.destination).map_err(super::internal)? {
            IndexMetadata::load(&build.destination).map_err(super::internal)?;
            return Ok(Status::Ready);
        }
        if build.destination.exists() {
            // Only remove the empty skeleton previously verified above. remove_dir
            // refuses nonempty directories, including a writer's newly added files.
            if build.destination.join("tantivy").is_dir() { fs::remove_dir(build.destination.join("tantivy")).map_err(super::internal)?; }
            fs::remove_dir(&build.destination).map_err(super::internal)?;
        }
        if let crate::storage::DataSource::Tantivy { path, .. } = &mut metadata.data_source { *path = build.destination.join("tantivy"); }
        metadata.save(&staged).map_err(super::internal)?;
        fs::rename(&staged, &build.destination).map_err(super::internal)?;
        #[cfg(unix)]
        File::open(build.destination.parent().ok_or_else(|| super::internal("Index has no parent"))?).and_then(|file| file.sync_all()).map_err(super::internal)?;
        Ok(Status::Ready)
    }).await
}

fn plan(workspace: &Workspace, ct: &CancellationToken) -> Result<Plan, ErrorData> {
    let mut settings = read_settings(&workspace.root).map_err(super::internal)?;
    let destination = confined_index_path(&workspace.root, &settings).map_err(super::internal)?;
    if destination.join("index.meta").is_file() { IndexMetadata::load(&destination).map_err(super::internal)?; return Ok(Plan::Complete(Status::Ready)); }
    if !crate::cli::automatic::is_uninitialized_index(&destination).map_err(super::internal)? { return Err(super::internal("Existing index is incomplete. Automatic setup will not replace it; run codanna index for recovery.")); }
    let state = workspace.root.join(crate::init::local_dir_name());
    let lock_path = state.join("bootstrap.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.is_symlink()) { return Err(super::internal("Bootstrap lock must not be a symlink")); }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path).map_err(super::internal)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(Plan::Complete(Status::Busy)),
        Err(error) => return Err(super::internal(error)),
    }
    if destination.join("index.meta").is_file() { return Ok(Plan::Complete(Status::Ready)); }
    let mut roots = Vec::new();
    for source in &settings.indexing.indexed_paths {
        let path = workspace.root.join(source).canonicalize().map_err(super::internal)?;
        if !path.starts_with(&workspace.root) { return Err(super::internal("Automatic indexing cannot include external source roots")); }
        roots.push(path);
    }
    if roots.is_empty() { roots.push(workspace.root.clone()); }
    let mut walker = ignore::WalkBuilder::new(&roots[0]);
    for root in roots.iter().skip(1) { walker.add(root); }
    walker.follow_links(false).parents(false).git_global(false).require_git(false).add_custom_ignore_filename(".codannaignore");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut files = 0usize;
    let mut entries = 0usize;
    let mut bytes = 0u64;
    for entry in walker.build() {
        entries += 1;
        if ct.is_cancelled() || Instant::now() >= deadline || entries > 100_000 { return Err(super::internal("Initial source discovery exceeded its budget; use explicit codanna index for this workspace")); }
        let entry = entry.map_err(super::internal)?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            files += 1;
            bytes = bytes.saturating_add(entry.metadata().map_err(super::internal)?.len());
            if files > 50_000 || bytes > 512 * 1024 * 1024 { return Err(super::internal("Workspace exceeds automatic indexing limits (50,000 files / 512 MiB); use explicit codanna index")); }
        }
    }
    if files == 0 { return Ok(Plan::Complete(Status::Empty)); }
    let staging = tempfile::Builder::new().prefix(".bootstrap-").tempdir_in(&state).map_err(super::internal)?;
    settings.workspace_root = Some(workspace.root.clone());
    settings.index_path = staging.path().join("index");
    settings.indexing.indexed_paths = roots;
    settings.indexing.parallelism = 2;
    settings.semantic_search.enabled = false;
    settings.file_watch.enabled = false;
    let config = staging.path().join("settings.toml");
    fs::write(&config, toml::to_string_pretty(&settings).map_err(super::internal)?).map_err(super::internal)?;
    Ok(Plan::Build(Build { _lock: lock, staging, config, destination, root: workspace.root.clone() }))
}

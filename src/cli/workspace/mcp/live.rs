//! Native freshness for automatically created code-only workspaces. One elected
//! writer per physical index; other local connections are read-only followers.
use crate::indexing::facade::IndexFacade;
use crate::mcp::notifications::NotificationBroadcaster;
use crate::storage::{IndexMetadata, IndexPersistence, write_lease::CodeWriteLease};
use crate::watcher::{UnifiedWatcher, handlers::CodeFileHandler};
use crate::{IndexError, Settings};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

type Facade = Arc<RwLock<IndexFacade>>;

#[derive(Clone, Serialize)]
pub(super) struct State {
    pub status: &'static str,
    pub ready: bool,
    pub detail: Option<String>,
}
impl State {
    fn new(status: &'static str, ready: bool) -> Self {
        Self {
            status,
            ready,
            detail: None,
        }
    }
}

pub(super) struct LiveWorkspace {
    state: Arc<Mutex<State>>,
    stop: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
}
impl LiveWorkspace {
    pub(super) fn start(facade: Facade, settings: Arc<Settings>) -> Arc<Self> {
        let state = Arc::new(Mutex::new(State::new("starting", false)));
        let stop = CancellationToken::new();
        let state_task = state.clone();
        let token = stop.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = run(facade, settings, state_task.clone(), token).await {
                *state_task.lock() = State {
                    status: "unavailable",
                    ready: false,
                    detail: Some(error.to_string()),
                };
            }
        });
        Arc::new(Self {
            state,
            stop,
            task: Mutex::new(Some(task)),
        })
    }
    pub(super) fn state(&self) -> State {
        self.state.lock().clone()
    }
    pub(super) async fn shutdown(&self) {
        self.stop.cancel();
        let task = { self.task.lock().take() };
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}
impl Drop for LiveWorkspace {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[derive(PartialEq, Eq, Clone)]
struct ContextStamp {
    config: (u64, SystemTime),
    ignore: Option<(u64, SystemTime)>,
}
fn context_stamp(root: &Path) -> Result<ContextStamp, IndexError> {
    fn stamp(path: &Path) -> Result<Option<(u64, SystemTime)>, IndexError> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => Ok(Some((metadata.len(), metadata.modified()?))),
            Ok(_) => Err(IndexError::General(
                "Workspace settings/ignore file must be regular".into(),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    Ok(ContextStamp {
        config: stamp(&crate::cli::automatic::config_path(root))?
            .ok_or_else(|| IndexError::General("Workspace configuration disappeared".into()))?,
        ignore: stamp(&root.join(".codannaignore"))?,
    })
}

pub(super) fn eligible(settings: &Settings) -> bool {
    settings.file_watch.enabled
        && !settings.semantic_search.enabled
        && !settings.index_path.join("semantic/metadata.json").exists()
}

async fn run(
    facade: Facade,
    settings: Arc<Settings>,
    state: Arc<Mutex<State>>,
    stop: CancellationToken,
) -> Result<(), IndexError> {
    let owned = settings.clone();
    let automatic = crate::runtime::blocking(move || eligible(&owned)).await?;
    if !automatic {
        *state.lock() = State::new(
            if settings.file_watch.enabled {
                "semantic-manual"
            } else {
                "disabled"
            },
            true,
        );
        return Ok(());
    }
    let root = settings
        .workspace_root
        .clone()
        .ok_or_else(|| IndexError::General("Missing live workspace root".into()))?;
    let owned = root.clone();
    let initial = crate::runtime::blocking(move || context_stamp(&owned)).await??;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            _ = tick.tick() => {},
        }
        let index = settings.index_path.clone();
        let checked = root.clone();
        let expected = initial.clone();
        let lease = crate::runtime::blocking(move || {
            if context_stamp(&checked)? != expected {
                return Err(IndexError::General(
                    "Workspace configuration changed; the router must reload this reader".into(),
                ));
            }
            CodeWriteLease::try_acquire(&index)
        })
        .await??;
        let Some(lease) = lease else {
            *state.lock() = State::new("following", true);
            continue;
        };
        // A writer could have replaced the index while this reader was a
        // follower. Reload strictly after election, never construct an empty one.
        *state.lock() = State::new("refreshing", false);
        let owned = settings.clone();
        let workspace = root.clone();
        let lease_for_load = lease.clone();
        crate::runtime::mutate(&facade, move |index| {
            let _lease = lease_for_load;
            if !eligible(&owned) {
                return Err(IndexError::General(
                    "Semantic configuration changed; reload the workspace reader".into(),
                ));
            }
            let metadata = IndexMetadata::load(&owned.index_path)?;
            if metadata.emission_version != Some(crate::storage::EMISSION_SEMANTICS_VERSION) {
                return Err(IndexError::General(
                    "Index format changed; an explicit rebuild is required".into(),
                ));
            }
            let mut refreshed =
                IndexPersistence::new(owned.index_path.clone()).load_facade_lite(owned)?;
            refreshed.restrict_workspace(workspace)?;
            *index = refreshed;
            Ok::<_, IndexError>(())
        })
        .await??;
        // Catch up offline edits, install native watches, then close the scan /
        // registration race once. Warm queries never run either discovery pass.
        catch_up(&facade, &settings, &root, &initial, &stop, lease.clone()).await?;
        let broadcaster = Arc::new(NotificationBroadcaster::new(32));
        let mut watcher = UnifiedWatcher::builder()
            .broadcaster(broadcaster)
            .indexer(facade.clone())
            .index_path(settings.index_path.clone())
            .workspace_root(root.clone())
            .debounce_ms(settings.file_watch.debounce_ms)
            .handler(CodeFileHandler::new(facade.clone(), root.clone()))
            .build()
            .map_err(|error| IndexError::General(error.to_string()))?;
        watcher
            .prepare()
            .await
            .map_err(|error| IndexError::General(error.to_string()))?;
        catch_up(&facade, &settings, &root, &initial, &stop, lease.clone()).await?;
        *state.lock() = State::new("watching", true);
        let watch_stop = stop.child_token();
        let watch = watcher.watch_until(watch_stop.clone());
        tokio::pin!(watch);
        let outcome = loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break Ok(()),
                result = &mut watch => return result.map_err(|error| IndexError::General(error.to_string())),
                _ = tick.tick() => {
                    let checked = root.clone();
                    let current = crate::runtime::blocking(move || context_stamp(&checked)).await.and_then(|value| value);
                    match current {
                        Ok(current) if current == initial => {},
                        _ => break Err(IndexError::General("Workspace settings or ignore rules changed; reload the reader before further queries".into())),
                    }
                }
            }
        };
        watch_stop.cancel();
        // Do not abort this future: it can own a blocking mutation. The lease is
        // retained through the actual write completion, not just waiter lifetime.
        watch
            .await
            .map_err(|error| IndexError::General(error.to_string()))?;
        drop(lease);
        return outcome;
    }
}

async fn catch_up(
    facade: &Facade,
    settings: &Arc<Settings>,
    root: &Path,
    expected: &ContextStamp,
    stop: &CancellationToken,
    lease: Arc<CodeWriteLease>,
) -> Result<(), IndexError> {
    let settings = settings.clone();
    let root = root.to_path_buf();
    let expected = expected.clone();
    let stop = stop.clone();
    crate::runtime::mutate(facade, move |index| {
        let _lease = lease;
        if stop.is_cancelled() {
            return Ok(());
        }
        if context_stamp(&root)? != expected {
            return Err(IndexError::General(
                "Workspace configuration changed during catch-up".into(),
            ));
        }
        let roots: Vec<PathBuf> = if settings.indexed_paths_cache.is_empty() {
            vec![root.clone()]
        } else {
            settings.indexed_paths_cache.clone()
        };
        for source in &roots {
            if source.canonicalize()? != *source || !source.starts_with(&root) {
                return Err(IndexError::General(
                    "Live source escaped the workspace".into(),
                ));
            }
        }
        // Complete conservative admission before any deletions; false means no
        // supported files remain and still needs the deletion reconciliation.
        super::bootstrap::discovery::has_sources(&settings, &roots, &stop)
            .map_err(|error| IndexError::General(error.to_string()))?;
        let mut pending = crate::indexing::pipeline::PendingResolution::default();
        for source in roots {
            if stop.is_cancelled() {
                break;
            }
            index.index_directory_deferred(&source, false, &mut pending)?;
        }
        index.resolve_deferred(pending)?;
        IndexPersistence::new(settings.index_path.clone()).save_facade(index)?;
        Ok::<_, IndexError>(())
    })
    .await?
}

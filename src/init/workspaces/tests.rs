use super::*;
use crate::cli::workspace::WorkspaceLaunch;
use std::ffi::OsString;
use tempfile::TempDir;

fn fixture(dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    fs::create_dir_all(root.join(local_dir_name())).unwrap();
    fs::write(
        root.join(local_dir_name()).join("settings.toml"),
        "version = 1\n[semantic_search]\nenabled = false\n",
    )
    .unwrap();
    root.canonicalize().unwrap()
}

fn registry(dir: &Path) -> WorkspaceRegistry {
    WorkspaceRegistry::new(dir.join("global/projects.json"))
}

#[test]
fn workspace_reads_do_not_create_global_state() {
    let temp = TempDir::new().unwrap();
    let registry = registry(temp.path());
    assert!(registry.list().unwrap().is_empty());
    assert!(registry.get("missing").is_err());
    assert!(!temp.path().join("global").exists());
}

#[test]
fn workspace_discovery_keeps_configured_members_in_one_workspace() {
    let temp = TempDir::new().unwrap();
    let primary = fixture(temp.path(), "workspace-a");
    let unrelated = fixture(temp.path(), "workspace-b");
    let nested = primary.join("repo-b/src");
    fs::create_dir_all(&nested).unwrap();
    let registry = registry(temp.path());
    let first = registry.add(&primary, Some("workspace-a")).unwrap();
    assert_eq!(registry.add(&nested, None).unwrap().id, first.id);
    registry.add(&unrelated, Some("workspace-b")).unwrap();
    let names: Vec<_> = registry
        .list()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert_eq!(names, ["workspace-a", "workspace-b"]);
    assert!(!primary.join(local_dir_name()).join("index").exists());
    assert!(!nested.join(local_dir_name()).exists());
}

#[test]
fn workspace_cache_only_directory_is_not_a_configuration() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(local_dir_name()).join("models")).unwrap();
    let source = temp.path().join("uninitialized/src");
    fs::create_dir_all(&source).unwrap();
    assert!(discover(&source).unwrap().is_none());
    assert!(registry(temp.path()).add(&source, None).is_err());
    assert!(!temp.path().join("global").exists());
}

#[test]
fn workspace_malformed_nearest_config_never_falls_through() {
    let temp = TempDir::new().unwrap();
    let outer = fixture(temp.path(), "outer");
    let inner = fixture(&outer, "inner");
    fs::write(
        inner.join(local_dir_name()).join("settings.toml"),
        "[broken",
    )
    .unwrap();
    assert!(discover(&inner).is_err());
    assert!(registry(temp.path()).add(&inner, None).is_err());
}

#[test]
fn workspace_alias_collisions_fail_without_overwriting() {
    let temp = TempDir::new().unwrap();
    let a = fixture(temp.path(), "a");
    let b = fixture(temp.path(), "b");
    let registry = registry(temp.path());
    let first = registry.add(&a, Some("workspace")).unwrap();
    let original = fs::read(&registry.path).unwrap();
    assert!(registry.add(&b, Some("workspace")).is_err());
    assert!(registry.add(&b, Some(first.id.as_str())).is_err());
    assert!(registry.add(&a, Some("silently-renamed")).is_err());
    assert_eq!(fs::read(&registry.path).unwrap(), original);
}

#[test]
fn workspace_reinitialization_preserves_id_alias_and_counters() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "project");
    let registry = registry(temp.path());
    let first = registry.add(&root, Some("workspace-a")).unwrap();
    with_registry(&registry.path, |state| {
        state
            .projects
            .get_mut(first.id.as_str())
            .unwrap()
            .symbol_count = 123;
        Ok(())
    })
    .unwrap();
    for update in [true, false] {
        let id = ProjectRegistry::register_at(&registry.path, &root, update).unwrap();
        assert_eq!(id, first.id.as_str());
    }
    let state = ProjectRegistry::load_from_path(&registry.path).unwrap();
    assert_eq!(state.projects[first.id.as_str()].name, "workspace-a");
    assert_eq!(state.projects[first.id.as_str()].symbol_count, 123);
}

#[test]
fn workspace_corrupt_or_future_registry_is_preserved() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "project");
    let registry = registry(temp.path());
    fs::create_dir_all(registry.path.parent().unwrap()).unwrap();
    for original in [
        "broken JSON",
        r#"{"version":99,"projects":{},"default_project":null}"#,
    ] {
        fs::write(&registry.path, original).unwrap();
        assert!(registry.add(&root, None).is_err());
        assert!(registry.list().is_err());
        assert_eq!(fs::read_to_string(&registry.path).unwrap(), original);
    }
}

#[test]
fn workspace_stale_snapshot_cannot_overwrite_another_registration() {
    let temp = TempDir::new().unwrap();
    let a = fixture(temp.path(), "a");
    let b = fixture(temp.path(), "b");
    let registry = registry(temp.path());
    registry.add(&a, None).unwrap();
    let snapshot = ProjectRegistry::load_from_path(&registry.path).unwrap();
    registry.add(&b, None).unwrap();
    let original = fs::read(&registry.path).unwrap();
    assert!(snapshot.save_snapshot_to_path(&registry.path).is_err());
    assert_eq!(fs::read(&registry.path).unwrap(), original);
    assert_eq!(registry.list().unwrap().len(), 2);
}

#[test]
fn workspace_remove_and_rename_never_touch_project_data() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "a");
    let index = root.join(local_dir_name()).join("index");
    fs::create_dir_all(&index).unwrap();
    fs::write(index.join("sentinel"), "keep me").unwrap();
    let config = fs::read(root.join(local_dir_name()).join("settings.toml")).unwrap();
    let registry = registry(temp.path());
    let before = registry.add(&root, None).unwrap();
    let after = registry.rename("a", "renamed").unwrap();
    assert_eq!(before.id, after.id);
    registry.remove("renamed").unwrap();
    assert!(registry.list().unwrap().is_empty());
    assert_eq!(fs::read(index.join("sentinel")).unwrap(), b"keep me");
    assert_eq!(
        fs::read(root.join(local_dir_name()).join("settings.toml")).unwrap(),
        config
    );
}

#[test]
fn workspace_move_requires_original_to_be_gone_and_preserves_id() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "a");
    let copy = fixture(temp.path(), "copy");
    let registry = registry(temp.path());
    let before = registry.add(&root, None).unwrap();
    assert!(registry.relocate("a", &copy).is_err());
    let destination = temp.path().join("moved");
    fs::rename(&root, &destination).unwrap();
    let after = registry.relocate("a", &destination).unwrap();
    assert_eq!(before.id, after.id);
    assert_eq!(after.name, "a");
    assert_eq!(after.root, destination.canonicalize().unwrap());
}

#[test]
fn workspace_copied_root_or_external_index_fails_closed() {
    let temp = TempDir::new().unwrap();
    let a = fixture(temp.path(), "a");
    let b = fixture(temp.path(), "b");
    let config = a.join(local_dir_name()).join("settings.toml");
    fs::write(
        &config,
        toml::to_string(&serde_json::json!({"workspace_root": b})).unwrap(),
    )
    .unwrap();
    assert!(read_settings(&a).is_err());
    fs::write(
        &config,
        toml::to_string(&serde_json::json!({"index_path": b.join(local_dir_name()).join("index")}))
            .unwrap(),
    )
    .unwrap();
    assert!(registry(temp.path()).add(&a, None).is_err());
    assert!(!b.join(local_dir_name()).join("index").exists());
}

#[test]
fn workspace_launch_is_explicit_and_does_not_change_process_state() {
    let temp = TempDir::new().unwrap();
    let a = fixture(temp.path(), "workspace-a");
    let registry = registry(temp.path());
    registry.add(&a, None).unwrap();
    let cwd = std::env::current_dir().unwrap();
    let args: Vec<_> = ["--workspace", "workspace-a", "serve"]
        .iter()
        .map(|value| OsString::from(*value))
        .collect();
    let plan = WorkspaceLaunch::prepare(&registry, "workspace-a", &args).unwrap();
    let command = plan.command(Path::new("codanna-fixture"));
    assert_eq!(command.get_current_dir(), Some(a.as_path()));
    let command_args: Vec<_> = command.get_args().map(OsString::from).collect();
    assert_eq!(
        command_args,
        vec![
            OsString::from("--config"),
            a.join(local_dir_name())
                .join("settings.toml")
                .into_os_string(),
            OsString::from("serve")
        ]
    );
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "CODANNA_RECALL_WORKSPACE" && value.is_none())
    );
    assert_eq!(std::env::current_dir().unwrap(), cwd);
    assert!(!a.join(local_dir_name()).join("index").exists());
}

#[cfg(unix)]
#[test]
fn workspace_symlink_roots_deduplicate_but_index_escape_fails() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "real");
    let link = temp.path().join("linked");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    let registry = registry(temp.path());
    let real = registry.add(&root, None).unwrap();
    assert_eq!(registry.add(&link, None).unwrap().id, real.id);
    let outside = temp.path().join("outside-index");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join(local_dir_name()).join("index")).unwrap();
    assert!(confined_index_path(&root, &read_settings(&root).unwrap()).is_err());
}

#[test]
fn workspace_doctor_does_not_claim_index_completion() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "a");
    let registry = registry(temp.path());
    registry.add(&root, None).unwrap();
    let diagnostic = registry.doctor("a").unwrap();
    assert_eq!(diagnostic.status, "configured");
    assert!(!diagnostic.index_directory_exists);
    fs::remove_dir_all(&root).unwrap();
    let diagnostic = registry.doctor("a").unwrap();
    assert_eq!(diagnostic.status, "unavailable");
    assert!(diagnostic.detail.is_some());
    assert_eq!(registry.list().unwrap().len(), 1);
}

#[test]
fn workspace_launch_rejects_partial_rebuild_and_external_sources() {
    let temp = TempDir::new().unwrap();
    let root = fixture(temp.path(), "workspace-a");
    let outside = fixture(temp.path(), "workspace-b");
    fs::create_dir(root.join("src")).unwrap();
    let registry = registry(temp.path());
    registry.add(&root, None).unwrap();
    let config_path = root.join(local_dir_name()).join("settings.toml");
    let config_before = fs::read(&config_path).unwrap();
    let index = confined_index_path(&root, &read_settings(&root).unwrap()).unwrap();
    let full_root: Vec<_> = ["--workspace", "workspace-a", "index", ".", "--force"]
        .into_iter()
        .map(OsString::from)
        .collect();

    // First-time full-root setup is the only explicit-path exception. Merely
    // preparing it must not create an index or edit the user's configuration.
    assert!(WorkspaceLaunch::prepare(&registry, "workspace-a", &full_root).is_ok());
    assert!(!index.exists());
    for paths in [vec!["src"], vec![".", "src"]] {
        let partial: Vec<_> = ["--workspace", "workspace-a", "index", "--force"]
            .into_iter()
            .chain(paths)
            .map(OsString::from)
            .collect();
        assert!(WorkspaceLaunch::prepare(&registry, "workspace-a", &partial).is_err());
    }
    let external = vec![
        OsString::from("--workspace"),
        OsString::from("workspace-a"),
        OsString::from("add-dir"),
        outside.into_os_string(),
    ];
    assert!(WorkspaceLaunch::prepare(&registry, "workspace-a", &external).is_err());
    assert!(!index.exists());
    assert_eq!(fs::read(&config_path).unwrap(), config_before);

    // Once any data exists, even the same full-root --force command is refused.
    // This preserves the original test's destructive-rebuild boundary.
    fs::create_dir_all(&index).unwrap();
    fs::write(index.join("sentinel"), b"existing workspace data").unwrap();
    assert!(WorkspaceLaunch::prepare(&registry, "workspace-a", &full_root).is_err());
    assert_eq!(
        fs::read(index.join("sentinel")).unwrap(),
        b"existing workspace data"
    );
    assert_eq!(fs::read(&config_path).unwrap(), config_before);
}

#[test]
fn workspace_legacy_duplicate_alias_requires_exact_id() {
    let temp = TempDir::new().unwrap();
    let registry = registry(temp.path());
    let a = fixture(temp.path(), "one");
    let b = fixture(temp.path(), "two");
    let first = registry.add(&a, None).unwrap();
    let second = registry.add(&b, None).unwrap();
    with_registry(&registry.path, |state| {
        state.projects.get_mut(first.id.as_str()).unwrap().name = "legacy".into();
        state.projects.get_mut(second.id.as_str()).unwrap().name = "legacy".into();
        Ok(())
    })
    .unwrap();
    assert!(registry.get("legacy").is_err());
    assert_eq!(registry.get(first.id.as_str()).unwrap().root, a);
    registry.rename(second.id.as_str(), "unique").unwrap();
    assert_eq!(registry.get("legacy").unwrap().id, first.id);
}

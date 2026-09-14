use super::*;
use clap::Parser;
use tempfile::TempDir;

fn configuration(root: &Path, roots: &[&str]) {
    fs::create_dir_all(root.join(crate::init::local_dir_name())).unwrap();
    let mut settings = Settings::default();
    settings.semantic_search.enabled = false;
    settings.index_path = PathBuf::from(crate::init::local_dir_name()).join("index");
    settings.indexing.indexed_paths = roots.iter().map(PathBuf::from).collect();
    fs::write(
        config_path(root),
        toml::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
}

fn checkout(path: &Path) {
    fs::create_dir_all(path.join(".git")).unwrap();
    fs::create_dir_all(path.join("src")).unwrap();
}

fn registry(temp: &TempDir) -> WorkspaceRegistry {
    WorkspaceRegistry::new(temp.path().join("home/projects.json"))
}

#[test]
fn workspace_auto_first_index_prepares_without_models_or_indexes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace-a");
    checkout(&root.join("repo-a"));
    checkout(&root.join("repo-b"));
    let registry = registry(&temp);
    let workspace = prepare(&registry, &root, None, StartupMode::Index).unwrap();
    assert_eq!(workspace.name, "workspace-a");
    assert_eq!(
        read_settings(&root).unwrap().indexing.indexed_paths,
        [PathBuf::from(".")]
    );
    assert!(
        !root
            .join(crate::init::local_dir_name())
            .join("index")
            .exists()
    );
    assert!(!temp.path().join("home/models").exists());
    assert_eq!(registry.list().unwrap().len(), 1);
}

#[test]
fn workspace_auto_git_subdirectory_selects_checkout_not_source_folder() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace-b");
    checkout(&root);
    let discovery = inspect(&root.join("src"), None).unwrap();
    assert_eq!(discovery.root, root.canonicalize().unwrap());
    assert_eq!(discovery.reason, "nearest-git-checkout");
    assert!(!config_path(&root).exists());
}

#[test]
fn workspace_auto_parent_owns_members_even_with_legacy_child_configs() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().join("workspace-a");
    let first = parent.join("repo-a");
    let second = parent.join("repo-b");
    checkout(&first);
    checkout(&second);
    configuration(&parent, &["repo-a", "repo-b"]);
    configuration(&first, &["src"]);
    configuration(&second, &["src"]);
    let child_bytes = fs::read(config_path(&second)).unwrap();
    let registry = registry(&temp);
    let from_second = prepare(&registry, &second.join("src"), None, StartupMode::Index).unwrap();
    let from_first = prepare(&registry, &first, None, StartupMode::Index).unwrap();
    assert_eq!(from_second.id, from_first.id);
    assert_eq!(from_second.root, parent.canonicalize().unwrap());
    assert_eq!(registry.list().unwrap().len(), 1);
    assert_eq!(fs::read(config_path(&second)).unwrap(), child_bytes);
}

#[test]
fn workspace_auto_parent_does_not_claim_unlisted_child() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().join("projects");
    let child = parent.join("independent");
    checkout(&child);
    fs::create_dir_all(parent.join("other")).unwrap();
    configuration(&parent, &["other"]);
    configuration(&child, &["src"]);
    assert_eq!(
        inspect(&child, None).unwrap().root,
        child.canonicalize().unwrap()
    );
}

#[test]
fn workspace_auto_independent_siblings_are_not_combined() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("projects/workspace-a");
    let second = temp.path().join("projects/workspace-b");
    checkout(&first);
    checkout(&second);
    assert_eq!(
        inspect(&first.join("src"), None).unwrap().root,
        first.canonicalize().unwrap()
    );
    assert_eq!(
        inspect(&second.join("src"), None).unwrap().root,
        second.canonicalize().unwrap()
    );
    assert!(!config_path(&temp.path().join("projects")).exists());
}

#[test]
fn workspace_auto_preserves_configuration_and_existing_registration_alias() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("actual-directory");
    checkout(&root);
    configuration(&root, &["src"]);
    let bytes = fs::read(config_path(&root)).unwrap();
    let registry = registry(&temp);
    let original = registry.add(&root, Some("custom-alias")).unwrap();
    let selected = prepare(&registry, &root, None, StartupMode::Index).unwrap();
    assert_eq!(selected.id, original.id);
    assert_eq!(selected.name, "custom-alias");
    assert_eq!(fs::read(config_path(&root)).unwrap(), bytes);
}

#[test]
fn workspace_auto_disambiguates_equal_directory_names() {
    let temp = TempDir::new().unwrap();
    let one = temp.path().join("one/backend");
    let two = temp.path().join("two/backend");
    checkout(&one);
    checkout(&two);
    let registry = registry(&temp);
    let a = prepare(&registry, &one, None, StartupMode::Index).unwrap();
    let b = prepare(&registry, &two, None, StartupMode::Index).unwrap();
    assert_ne!(a.id, b.id);
    assert_ne!(a.name, b.name);
    assert_eq!(
        prepare(&registry, &two, None, StartupMode::Index)
            .unwrap()
            .id,
        b.id
    );
}

#[test]
fn workspace_auto_existing_mode_never_bootstraps_missing_index() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace-b");
    checkout(&root);
    let registry = registry(&temp);
    assert!(prepare(&registry, &root, None, StartupMode::Existing).is_err());
    assert!(!config_path(&root).exists());
    configuration(&root, &["src"]);
    assert!(prepare(&registry, &root, None, StartupMode::Existing).is_err());
    assert!(!temp.path().join("home").exists());
}

#[test]
fn workspace_auto_corrupt_registry_prevents_initialization() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace-b");
    checkout(&root);
    let registry_path = temp.path().join("broken.json");
    fs::write(&registry_path, b"not json").unwrap();
    let registry = WorkspaceRegistry::new(registry_path.clone());
    assert!(prepare(&registry, &root, None, StartupMode::Index).is_err());
    assert!(!config_path(&root).exists());
    assert_eq!(fs::read(registry_path).unwrap(), b"not json");
}

#[test]
fn workspace_auto_malformed_nearest_config_cannot_fall_back_to_parent() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().join("workspace-a");
    let child = parent.join("repo-a");
    checkout(&child);
    configuration(&parent, &["."]);
    configuration(&child, &["src"]);
    fs::write(config_path(&child), b"[invalid").unwrap();
    assert!(inspect(&child.join("src"), None).is_err());
}

#[test]
fn workspace_auto_rejects_home_and_filesystem_root() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    checkout(&home.join("project"));
    assert!(inspect(&home, Some(&home)).is_err());
    let canonical = home.canonicalize().unwrap();
    assert!(inspect(canonical.ancestors().last().unwrap(), None).is_err());
    assert!(!config_path(&home).exists());
}

#[test]
fn workspace_auto_cache_only_ancestor_is_not_a_workspace() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(
        temp.path()
            .join(crate::init::local_dir_name())
            .join("models"),
    )
    .unwrap();
    let root = temp.path().join("project");
    checkout(&root);
    assert_eq!(
        inspect(&root, None).unwrap().root,
        root.canonicalize().unwrap()
    );
}

#[test]
fn workspace_auto_inventory_is_bounded_and_skips_dependencies() {
    let temp = TempDir::new().unwrap();
    checkout(temp.path());
    checkout(&temp.path().join("node_modules/hidden"));
    checkout(&temp.path().join("vendor/hidden"));
    checkout(&temp.path().join("web"));
    let deep = temp.path().join("a/b/c/d/e/f/g/h");
    checkout(&deep);
    let discovery = inspect(temp.path(), None).unwrap();
    assert!(discovery.inventory_truncated);
    let paths: Vec<_> = discovery
        .repositories
        .iter()
        .map(|r| r.path.as_path())
        .collect();
    assert!(paths.contains(&Path::new(".")));
    assert!(paths.contains(&Path::new("web")));
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with("vendor") || p.starts_with("node_modules"))
    );
}

#[test]
fn workspace_auto_worktree_marker_is_not_followed_outside_checkout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("worktree");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join(".git"),
        "gitdir: /unavailable/common/git/worktrees/demo\n",
    )
    .unwrap();
    assert_eq!(
        inspect(&root.join("src"), None).unwrap().root,
        root.canonicalize().unwrap()
    );
}

#[test]
fn workspace_auto_modes_exclude_network_explicit_config_and_dry_run() {
    for command in [
        vec!["codanna", "index", "--dry-run"],
        vec!["codanna", "index", "src"],
        vec!["codanna", "serve", "--http"],
        vec!["codanna", "--config", "settings.toml", "serve"],
        vec!["codanna", "--workspace", "workspace-a", "serve"],
        vec!["codanna", "completions", "zsh"],
    ] {
        assert!(startup_mode(&Cli::try_parse_from(command).unwrap()).is_none());
    }
    assert_eq!(
        startup_mode(&Cli::try_parse_from(["codanna", "index"]).unwrap()),
        Some(StartupMode::Index)
    );
    assert_eq!(
        startup_mode(&Cli::try_parse_from(["codanna", "serve"]).unwrap()),
        Some(StartupMode::Existing)
    );
    assert!(auto_setup_enabled(Some("false")).is_ok_and(|enabled| !enabled));
    assert!(auto_setup_enabled(Some("maybe")).is_err());
}

#[cfg(unix)]
#[test]
fn workspace_auto_symlinks_deduplicate_without_scanning_external_repos() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("workspace-a");
    checkout(&root);
    let external = temp.path().join("workspace-b");
    checkout(&external);
    symlink(&external, root.join("linked")).unwrap();
    let alias = temp.path().join("workspace-link");
    symlink(&root, &alias).unwrap();
    let discovery = inspect(&alias, None).unwrap();
    assert_eq!(discovery.root, root.canonicalize().unwrap());
    assert!(
        !discovery
            .repositories
            .iter()
            .any(|r| r.path == Path::new("linked"))
    );
}

#[test]
fn workspace_auto_unconfigured_nested_git_does_not_join_unlisted_parent() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().join("projects");
    let child = parent.join("independent");
    checkout(&child);
    fs::create_dir_all(parent.join("other")).unwrap();
    configuration(&parent, &["other"]);
    assert_eq!(
        inspect(&child, None).unwrap().root,
        child.canonicalize().unwrap()
    );
}

#[test]
fn workspace_auto_empty_init_skeleton_is_not_existing_data() {
    let temp = TempDir::new().unwrap();
    let index = temp.path().join("index");
    assert!(is_uninitialized_index(&index).unwrap());
    fs::create_dir_all(index.join("tantivy")).unwrap();
    assert!(is_uninitialized_index(&index).unwrap());
    fs::write(index.join("tantivy/meta.json"), "{}").unwrap();
    assert!(!is_uninitialized_index(&index).unwrap());
}

#[test]
fn workspace_auto_preserves_existing_ignore_rules() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    checkout(&root);
    fs::write(root.join(".codannaignore"), "secret/\n").unwrap();
    prepare(&registry(&temp), &root, None, StartupMode::Index).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".codannaignore")).unwrap(),
        "secret/\n"
    );
}

#[test]
fn workspace_auto_selection_is_independent_of_workspace_and_repository_names() {
    let temp = TempDir::new().unwrap();
    let registry = registry(&temp);
    for (index, name) in ["workspace-17", "Example.Space", "9_sandbox"]
        .iter()
        .enumerate()
    {
        let root = temp.path().join(name);
        let member = root.join(format!("member-{index}"));
        checkout(&member);
        let selected = prepare(&registry, &root, None, StartupMode::Index).unwrap();
        let alias = format!("alias-{index}");
        let renamed = registry.rename(selected.id.as_str(), &alias).unwrap();
        let nested = prepare(&registry, &member.join("src"), None, StartupMode::Index).unwrap();
        assert_eq!(nested.id, renamed.id);
        assert_eq!(nested.name, alias);
        assert_eq!(nested.root, root.canonicalize().unwrap());
        assert!(!member.join(crate::init::local_dir_name()).exists());
    }
    let workspaces = registry.list().unwrap();
    assert_eq!(workspaces.len(), 3);
    for (index, workspace) in workspaces.iter().enumerate() {
        assert_eq!(workspace.name, format!("alias-{index}"));
        assert!(
            workspaces
                .iter()
                .skip(index + 1)
                .all(|other| other.id != workspace.id)
        );
    }
}

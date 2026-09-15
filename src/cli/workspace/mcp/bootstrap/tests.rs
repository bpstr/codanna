use super::*;

fn generation(files: u32) -> IndexMetadata {
    IndexMetadata {
        file_count: files,
        emission_version: Some(crate::storage::metadata::EMISSION_SEMANTICS_VERSION),
        ..IndexMetadata::default()
    }
}

#[test]
fn hardening_workspace_bootstrap_requires_indexed_files_not_just_directory_contents() {
    assert!(matches!(
        validate_generation(&generation(0)).unwrap(),
        Status::Empty
    ));
    // Zero emitted symbols is legitimate; an indexed file still establishes a
    // generation. This test must not accidentally equate symbols with files.
    assert!(matches!(
        validate_generation(&generation(1)).unwrap(),
        Status::Ready
    ));
    assert!(matches!(
        validate_generation(&generation(MAX_FILES)).unwrap(),
        Status::Ready
    ));
    assert!(validate_generation(&generation(MAX_FILES + 1)).is_err());
    let mut stale = generation(1);
    stale.emission_version = None;
    assert!(validate_generation(&stale).is_err());
}

fn prepared_build(temp: &tempfile::TempDir, files: u32) -> Build {
    let root = temp.path().canonicalize().unwrap();
    let state = root.join(crate::init::local_dir_name());
    fs::create_dir(&state).unwrap();
    let source_config = state.join("settings.toml");
    let source_config_bytes = b"[semantic_search]\nenabled = false\n".to_vec();
    fs::write(&source_config, &source_config_bytes).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("bootstrap.lock"))
        .unwrap();
    let staging = tempfile::tempdir_in(&state).unwrap();
    fs::create_dir_all(staging.path().join("index/tantivy")).unwrap();
    // Metadata-only publication checks do not open Tantivy; real MCP tests
    // exercise actual generation readability separately.
    fs::write(staging.path().join("index/tantivy/meta.json"), "{}").unwrap();
    generation(files)
        .save(&staging.path().join("index"))
        .unwrap();
    Build {
        _lock: lock,
        config: staging.path().join("settings.toml"),
        staging,
        destination: state.join("index"),
        root,
        source_config,
        source_config_bytes,
    }
}

#[test]
fn hardening_workspace_bootstrap_empty_generation_is_not_published() {
    let temp = tempfile::tempdir().unwrap();
    let build = prepared_build(&temp, 0);
    let destination = build.destination.clone();
    assert!(matches!(
        publish(build, &CancellationToken::new()).unwrap(),
        Status::Empty
    ));
    assert!(!destination.exists());
}

#[test]
fn hardening_workspace_bootstrap_changed_configuration_cannot_publish_old_scope() {
    let temp = tempfile::tempdir().unwrap();
    let build = prepared_build(&temp, 1);
    let destination = build.destination.clone();
    fs::write(
        &build.source_config,
        "[indexing]\nindexed_paths = [\"different\"]\n",
    )
    .unwrap();
    assert!(publish(build, &CancellationToken::new()).is_err());
    assert!(!destination.exists());
}

#[test]
fn hardening_workspace_bootstrap_overflow_and_cancellation_preserve_existing_storage() {
    for cancelled in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let build = prepared_build(&temp, if cancelled { 1 } else { MAX_FILES + 1 });
        let destination = build.destination.clone();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"existing data").unwrap();
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        assert!(publish(build, &token).is_err());
        assert_eq!(
            fs::read(destination.join("sentinel")).unwrap(),
            b"existing data"
        );
    }
}

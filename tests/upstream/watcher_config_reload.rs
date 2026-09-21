use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tantivy::{Index, Term, collector::Count, query::TermQuery, schema::IndexRecordOption};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn settings(root: &Path, index_path: &Path, roots: &[PathBuf]) -> String {
    let roots = roots
        .iter()
        .map(|path| serde_json::to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "workspace_root = {}\nindex_path = {}\n[indexing]\nindexed_paths = [{roots}]\n[semantic_search]\nenabled = false\n",
        serde_json::to_string(root).unwrap(),
        serde_json::to_string(index_path).unwrap(),
    )
}

#[test]
#[ignore = "creates 5,000 directories; run explicitly to check config-reload scaling"]
fn watches_large_root_added_by_settings_reload() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let bootstrap = root.join("bootstrap");
    let added = root.join("added");
    let index_path = root.join(".codanna/index");
    let settings_path = root.join(".codanna/settings.toml");
    fs::create_dir_all(&bootstrap).unwrap();
    fs::create_dir_all(&added).unwrap();
    fs::create_dir(root.join(".codanna")).unwrap();
    fs::write(
        bootstrap.join("seed.py"),
        "def bootstrap_seed():\n    return 0\n",
    )
    .unwrap();
    fs::write(added.join("seed.py"), "def added_seed():\n    return 1\n").unwrap();
    for i in 0..5000 {
        let path = added.join(format!("d{i}"));
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("seed.py"),
            format!("def reloaded_seed_{i}():\n    return {i}\n"),
        )
        .unwrap();
    }
    fs::write(
        &settings_path,
        settings(&root, &index_path, std::slice::from_ref(&bootstrap)),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_codanna");
    assert!(
        Command::new(binary)
            .current_dir(&root)
            .args(["index", "--no-progress"])
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GOOGLE_API_KEY")
            .env_remove("CODANNA_EMBED_PROVIDER")
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );

    let index = Index::open_in_dir(index_path.join("tantivy")).unwrap();
    let reader = index.reader().unwrap();
    let name_field = index.schema().get_field("name").unwrap();
    let contains = |name: &str| {
        reader.reload().unwrap();
        reader
            .searcher()
            .search(
                &TermQuery::new(
                    Term::from_field_text(name_field, name),
                    IndexRecordOption::Basic,
                ),
                &Count,
            )
            .unwrap()
            > 0
    };
    let wait_for = |condition: &dyn Fn() -> bool| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    };

    let log = root.join("watcher.log");
    let child = Command::new(binary)
        .current_dir(&root)
        .args(["serve", "--watch"])
        .env("RUST_LOG", "codanna::watcher::unified=info")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("CODANNA_EMBED_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let _child = ChildGuard(child);
    assert!(wait_for(&|| {
        fs::read_to_string(&log)
            .unwrap()
            .contains("[watcher] started")
    }));

    fs::write(
        &settings_path,
        settings(&root, &index_path, &[bootstrap, added.clone()]),
    )
    .unwrap();
    assert!(
        wait_for(&|| contains("added_seed")),
        "new root was not indexed after config reload: {}",
        fs::read_to_string(&log).unwrap()
    );

    fs::write(
        added.join("created.py"),
        "def created_after_reload():\n    return 2\n",
    )
    .unwrap();
    assert!(
        wait_for(&|| contains("created_after_reload")),
        "new file under reloaded root was not watched: {}",
        fs::read_to_string(log).unwrap()
    );
}

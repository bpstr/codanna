//! Incremental graph equivalence, persistence, and bounded-inventory controls.
//! Every fixture disables semantic search and uses only temporary local files.

#[test]
fn config_only_alias_change_rebinds_after_reopen_and_preserves_other_project() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("projects");
    let web = root.join("web");
    let mobile = root.join("mobile");
    for project in [&web, &mobile] {
        fs::create_dir_all(project).unwrap();
        fs::write(
            project.join("old.ts"),
            "export function work() { return 1; }\n",
        )
        .unwrap();
        fs::write(
            project.join("new.ts"),
            "export function work() { return 2; }\n",
        )
        .unwrap();
        fs::write(
            project.join("tsconfig.json"),
            r#"{"extends":"./base.json"}"#,
        )
        .unwrap();
        fs::write(
            project.join("base.json"),
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@work":["./old.ts"]}}}"#,
        )
        .unwrap();
    }
    fs::write(
        web.join("caller.ts"),
        "import { work } from '@work';\nexport function webCaller() { return work(); }\n",
    )
    .unwrap();
    fs::write(
        mobile.join("caller.ts"),
        "import { work } from '@work';\nexport function mobileCaller() { return work(); }\n",
    )
    .unwrap();
    let mut config = (*settings(&temp.path().join("index"), std::slice::from_ref(&root))).clone();
    config.languages.get_mut("typescript").unwrap().config_files =
        vec![web.join("tsconfig.json"), mobile.join("tsconfig.json")];
    let config = Arc::new(config);
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    index.index_directory(&root, false).unwrap();
    for caller in ["webCaller", "mobileCaller"] {
        let calls = index.get_called_functions(symbol(&index, caller).id);
        assert_eq!(calls.len(), 1);
        assert!(Path::new(calls[0].file_path.as_ref()).ends_with("old.ts"));
    }
    persistence.save_facade(&index).unwrap();
    drop(index);
    // Only inherited config changes; source bytes, hashes and mtimes stay fixed.
    fs::write(
        web.join("base.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@work":["./new.ts"]}}}"#,
    )
    .unwrap();
    let mut index = persistence.load_facade_lite(Arc::clone(&config)).unwrap();
    index.index_directory(&root, false).unwrap();
    let calls = index.get_called_functions(symbol(&index, "webCaller").id);
    assert_eq!(calls.len(), 1);
    assert!(Path::new(calls[0].file_path.as_ref()).ends_with("web/new.ts"));
    let calls = index.get_called_functions(symbol(&index, "mobileCaller").id);
    assert_eq!(calls.len(), 1);
    assert!(Path::new(calls[0].file_path.as_ref()).ends_with("mobile/old.ts"));
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .is_empty()
    );
    let stable = snapshot(&index);
    index.index_directory(&root, false).unwrap();
    assert_eq!(snapshot(&index), stable);
    fs::write(
        web.join("tsconfig.json"),
        r#"{"extends":"./base.json","compilerOptions":{"paths":{"@work":["./old.ts"]}}}"#,
    )
    .unwrap();
    index.index_directory(&root, false).unwrap();
    let calls = index.get_called_functions(symbol(&index, "webCaller").id);
    assert_eq!(calls.len(), 1);
    assert!(Path::new(calls[0].file_path.as_ref()).ends_with("web/old.ts"));
    let mut fresh_config = (*config).clone();
    fresh_config.index_path = temp.path().join("fresh");
    let mut fresh = IndexFacade::new(Arc::new(fresh_config)).unwrap();
    fresh.index_directory(&root, true).unwrap();
    assert_eq!(snapshot(&index), snapshot(&fresh));
}

use codanna::indexing::facade::IndexFacade;
use codanna::{IndexPersistence, RelationKind, Settings, Symbol};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

fn settings(index: &Path, roots: &[PathBuf]) -> Arc<Settings> {
    let mut settings = Settings {
        index_path: index.to_path_buf(),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    for root in roots {
        settings.add_indexed_path(root.clone()).unwrap();
    }
    Arc::new(settings)
}

fn facade(index: &Path, roots: &[PathBuf]) -> IndexFacade {
    IndexFacade::new(settings(index, roots)).unwrap()
}

fn symbol(index: &IndexFacade, name: &str) -> Symbol {
    let matches = index.find_symbols_by_name(name, None);
    assert_eq!(matches.len(), 1, "fixture requires unique {name}");
    matches[0].clone()
}

fn snapshot(index: &IndexFacade) -> (Vec<String>, Vec<String>) {
    let symbols = index.get_all_symbols();
    let mut source_paths = HashMap::new();
    for symbol in &symbols {
        source_paths.entry(symbol.file_id).or_insert_with(|| {
            // Symbol cards may render an absolute stored path relative to a
            // configured root. Compare the registration's source identity,
            // not that presentation choice, across fresh and restored facades.
            let stored = index
                .document_index()
                .get_file_path(symbol.file_id)
                .unwrap()
                .expect("every persisted symbol must have a live file registration");
            let stored = PathBuf::from(stored);
            if stored.is_absolute() {
                stored
            } else {
                index
                    .settings()
                    .workspace_root
                    .as_ref()
                    .expect("a relative registration requires its workspace root")
                    .join(stored)
            }
        });
    }
    let keys: HashMap<_, _> = symbols
        .iter()
        .map(|symbol| {
            (
                symbol.id,
                format!(
                    "{}:{}:{}:{:?}:{}:{:?}:{:?}:{:?}",
                    source_paths[&symbol.file_id].display(),
                    symbol.range.start_line,
                    symbol.range.start_column,
                    symbol.kind,
                    symbol.name,
                    symbol.signature,
                    symbol.module_path,
                    symbol.scope_context
                ),
            )
        })
        .collect();
    let mut declarations: Vec<_> = keys.values().cloned().collect();
    declarations.sort();
    let mut graph = Vec::new();
    for kind in [
        RelationKind::Calls,
        RelationKind::Uses,
        RelationKind::Defines,
        RelationKind::Extends,
        RelationKind::Implements,
        RelationKind::References,
    ] {
        for (from, to, _) in index
            .document_index()
            .get_all_relationships_by_kind(kind)
            .unwrap()
        {
            let source = keys
                .get(&from)
                .expect("every relationship source must be live");
            let target = keys
                .get(&to)
                .expect("every relationship target must be live");
            graph.push(format!("{source} --{kind:?}--> {target}"));
        }
    }
    graph.sort();
    (declarations, graph)
}

fn assert_fresh(index: &IndexFacade, roots: &[PathBuf], destination: &Path, state: &str) {
    let mut fresh = facade(destination, roots);
    fresh
        .index_directories_with_options(roots, false, false, false, None)
        .unwrap();
    assert_eq!(
        snapshot(index),
        snapshot(&fresh),
        "incremental/fresh mismatch at {state}"
    );
}

fn touch(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(300))
        .unwrap();
}

#[test]
fn multi_root_mutations_match_fresh_graph_in_both_orders() {
    for callee_first in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let tests = temp.path().join("tests");
        fs::create_dir_all(src.join("pkg")).unwrap();
        fs::create_dir(&tests).unwrap();
        fs::write(src.join("pkg/__init__.py"), "").unwrap();
        let target = src.join("pkg/mod.py");
        let moved = src.join("pkg/moved.py");
        let caller = tests.join("test_mod.py");
        let target_text = "def target_function():\n    return 42\n";
        let caller_text = "from pkg.mod import target_function\n\ndef test_target_function():\n    return target_function()\n";
        fs::write(&caller, caller_text).unwrap();
        let roots = if callee_first {
            vec![src, tests]
        } else {
            vec![tests, src]
        };
        let mut index = facade(&temp.path().join("index"), &roots);
        index
            .index_directories_with_options(&roots, false, false, false, None)
            .unwrap();
        assert!(
            index
                .get_called_functions(symbol(&index, "test_target_function").id)
                .is_empty()
        );
        assert_fresh(
            &index,
            &roots,
            &temp.path().join("fresh-missing"),
            "initial missing target",
        );

        for state in [
            "add",
            "edit-both",
            "delete",
            "restore",
            "rename",
            "retarget",
        ] {
            match state {
                "add" | "restore" => {
                    fs::write(&target, target_text).unwrap();
                    touch(&target);
                }
                "edit-both" => {
                    fs::write(&target, format!("# shifted\n{target_text}")).unwrap();
                    fs::write(&caller, format!("# shifted\n{caller_text}")).unwrap();
                    touch(&target);
                    touch(&caller);
                }
                "delete" => fs::remove_file(&target).unwrap(),
                "rename" => fs::rename(&target, &moved).unwrap(),
                "retarget" => {
                    fs::write(&caller, caller_text.replace("pkg.mod", "pkg.moved")).unwrap();
                    touch(&caller);
                }
                _ => unreachable!(),
            }
            index
                .index_directories_with_options(&roots, false, false, false, None)
                .unwrap();
            assert_fresh(
                &index,
                &roots,
                &temp.path().join(format!("fresh-{state}")),
                state,
            );
            if matches!(state, "add" | "restore" | "retarget") {
                assert_eq!(
                    index
                        .get_called_functions(symbol(&index, "test_target_function").id)
                        .len(),
                    1,
                    "fixture must resolve its available imported target at {state}, callee_first={callee_first}"
                );
            }
        }
    }
}

#[test]
fn unresolved_import_dependencies_survive_reopening() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("caller.ts"),
        "import { sharedTarget } from './target';\nexport function entry() { return sharedTarget(); }\n").unwrap();
    let config = settings(&temp.path().join("index"), std::slice::from_ref(&src));
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    index.index_directory(&src, false).unwrap();
    assert!(
        index
            .get_called_functions(symbol(&index, "entry").id)
            .is_empty()
    );
    persistence.save_facade(&index).unwrap();
    drop(index);

    let mut index = persistence.load_facade_lite(config).unwrap();
    fs::write(
        src.join("target.ts"),
        "export function sharedTarget() { return 42; }\n",
    )
    .unwrap();
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index.get_called_functions(symbol(&index, "entry").id).len(),
        1
    );
    assert_fresh(
        &index,
        &[src],
        &temp.path().join("fresh"),
        "reopened unresolved import",
    );
}

#[test]
fn persisted_dynamic_roots_retarget_import_only_barrels() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fixture");
    let pkg = root.join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("__init__.py"), "from pkg.a import helper\n").unwrap();
    fs::write(pkg.join("a.py"), "def helper(x):\n    return x + 1\n").unwrap();
    fs::write(pkg.join("b.py"), "def helper(x):\n    return x + 2\n").unwrap();
    fs::write(
        pkg.join("consumer.py"),
        "from pkg import helper\n\ndef entry(x):\n    return helper(x)\n",
    )
    .unwrap();
    // No configured roots: both the initial walk and restored metadata must
    // contribute to the namespace inventory used by subsequent file events.
    let config = settings(&temp.path().join("index"), &[]);
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    index
        .index_directory_with_options(&root, false, false, false, None)
        .unwrap();
    assert_eq!(
        index.get_called_functions(symbol(&index, "entry").id).len(),
        1
    );
    persistence.save_facade(&index).unwrap();
    drop(index);

    let mut index = persistence.load_facade_lite(config).unwrap();
    fs::write(pkg.join("__init__.py"), "from pkg.b import helper\n").unwrap();
    index.index_file(pkg.join("__init__.py")).unwrap();
    let calls = index.get_called_functions(symbol(&index, "entry").id);
    assert_eq!(calls.len(), 1);
    assert!(Path::new(calls[0].file_path.as_ref()).ends_with("b.py"));
    assert_fresh(
        &index,
        &[root],
        &temp.path().join("fresh"),
        "restored dynamic root",
    );
}

#[test]
fn unrelated_relative_importers_keep_their_symbol_ids() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("left")).unwrap();
    fs::create_dir_all(src.join("right")).unwrap();
    for (directory, name) in [("left", "leftEntry"), ("right", "rightEntry")] {
        fs::write(
            src.join(directory).join("target.ts"),
            "export function sharedTarget() { return 1; }\n",
        )
        .unwrap();
        fs::write(src.join(directory).join("caller.ts"), format!(
            "import {{ sharedTarget }} from './target';\nexport function {name}() {{ return sharedTarget(); }}\n")).unwrap();
    }
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    let unrelated = symbol(&index, "rightEntry").id;
    assert_eq!(index.get_called_functions(unrelated).len(), 1);
    fs::write(
        src.join("left/target.ts"),
        "export function sharedTarget() { return 2; }\n",
    )
    .unwrap();
    index.index_file(src.join("left/target.ts")).unwrap();
    assert_eq!(
        symbol(&index, "rightEntry").id,
        unrelated,
        "an unrelated relative importer must not be reparsed"
    );
    assert_fresh(
        &index,
        &[src],
        &temp.path().join("fresh"),
        "unrelated module isolation",
    );
}

#[test]
fn max_files_is_global_deterministic_and_zero_is_not_deletion_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("a.rs"), "pub fn first_a() {}\n").unwrap();
    fs::write(first.join("z.rs"), "pub fn first_z() {}\n").unwrap();
    fs::write(second.join("a.rs"), "pub fn second_a() {}\n").unwrap();
    let roots = vec![first.clone(), second];
    let mut index = facade(&temp.path().join("index"), &roots);
    let preview = index
        .index_directories_with_options(&roots, false, true, false, Some(1))
        .unwrap();
    assert_eq!(
        preview
            .iter()
            .map(|stats| stats.files_indexed)
            .sum::<usize>(),
        1
    );
    index
        .index_directories_with_options(&roots, false, false, false, Some(1))
        .unwrap();
    assert_eq!(
        index.get_all_indexed_paths(),
        vec![first.join("a.rs").canonicalize().unwrap()]
    );
    let before = snapshot(&index);
    fs::remove_file(first.join("a.rs")).unwrap();
    index
        .index_directories_with_options(&roots, false, false, false, Some(0))
        .unwrap();
    assert_eq!(
        snapshot(&index),
        before,
        "a zero-file inventory cannot establish a deletion"
    );
}

#[test]
fn bounded_barrel_update_invalidates_then_repairs_after_reopen_without_source_edits() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    let barrel = src.join("00_barrel.ts");
    let caller = src.join("z.ts");
    fs::write(&barrel, "export { work } from './a';\n").unwrap();
    fs::write(src.join("a.ts"), "export function work() { return 1; }\n").unwrap();
    fs::write(src.join("b.ts"), "export function work() { return 2; }\n").unwrap();
    fs::write(
        &caller,
        "import { work } from './00_barrel';\nexport function run() { return work(); }\n",
    )
    .unwrap();
    let config = settings(&temp.path().join("index"), std::slice::from_ref(&src));
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    index.index_directory(&src, false).unwrap();
    let before = index.get_called_functions(symbol(&index, "run").id);
    assert_eq!(before.len(), 1);
    assert!(Path::new(before[0].file_path.as_ref()).ends_with("a.ts"));
    let caller_id = symbol(&index, "run").id;
    fs::write(&barrel, "export { work } from './b';\n").unwrap();
    index
        .index_directory_with_options(&src, false, false, false, Some(1))
        .unwrap();
    assert_eq!(
        symbol(&index, "run").id,
        caller_id,
        "bounded update must not reopen the omitted caller"
    );
    assert!(
        index.get_called_functions(caller_id).is_empty(),
        "old a.ts edge must be invalidated"
    );
    assert_eq!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap(),
        vec![caller.canonicalize().unwrap()]
    );
    persistence.save_facade(&index).unwrap();
    drop(index);

    let mut index = persistence.load_facade_lite(config).unwrap();
    index
        .index_directory_with_options(&src, false, false, false, Some(0))
        .unwrap();
    assert_eq!(
        symbol(&index, "run").id,
        caller_id,
        "zero inventory must keep pending files unopened"
    );
    assert_eq!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap(),
        vec![caller.canonicalize().unwrap()]
    );
    // Every source hash is unchanged from its registration. Pending work must
    // independently trigger resolution through the ordinary directory API.
    index.index_directory(&src, false).unwrap();
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .is_empty()
    );
    let after = index.get_called_functions(symbol(&index, "run").id);
    assert_eq!(after.len(), 1);
    assert!(Path::new(after[0].file_path.as_ref()).ends_with("b.ts"));
    assert_fresh(
        &index,
        &[src],
        &temp.path().join("fresh"),
        "bounded barrel replay after reopening",
    );
}

#[test]
fn phase_one_publication_recovers_calls_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("a.rs"),
        "pub fn caller() { target(); }\npub fn target() {}\n",
    )
    .unwrap();
    let config = settings(&temp.path().join("index"), std::slice::from_ref(&src));
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    let mut pending = codanna::indexing::pipeline::PendingResolution::default();
    index
        .index_directory_deferred(&src, true, &mut pending)
        .unwrap();
    assert!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .is_empty()
    );
    assert_eq!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .len(),
        1
    );
    persistence.save_facade(&index).unwrap();
    drop(pending);
    drop(index);
    let mut index = persistence.load_facade_lite(config).unwrap();
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .len(),
        1
    );
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .is_empty()
    );
    assert_fresh(
        &index,
        &[src],
        &temp.path().join("fresh"),
        "Phase 1 restart",
    );
}

#[test]
fn recovery_retries_unregistered_outside_root_source_after_failed_parse() {
    use codanna::indexing::pipeline::stages::CleanupStage;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scanned = root.join("targets");
    let outside = root.join("callers");
    fs::create_dir(&scanned).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(scanned.join("target.rs"), "pub fn target() {}\n").unwrap();
    let caller = outside.join("caller.rs");
    let source = "pub fn caller() { target(); }\n";
    fs::write(&caller, source).unwrap();
    let roots = vec![scanned.clone(), outside.clone()];
    let config = settings(&temp.path().join("index"), &roots);
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    for root in &roots {
        index.index_directory(root, false).unwrap();
    }
    let mut pending = codanna::indexing::pipeline::PendingResolution::default();
    index
        .index_directory_deferred(&outside, true, &mut pending)
        .unwrap();
    drop(pending);
    let documents = Arc::clone(index.document_index());
    // Simulate process loss after cleanup and before replacement publication.
    CleanupStage::new(Arc::clone(&documents), config.index_path.join("semantic"))
        .cleanup_files_for_reindex(std::slice::from_ref(&caller))
        .unwrap();
    assert!(
        documents
            .get_file_info(&caller.to_string_lossy())
            .unwrap()
            .is_none()
    );
    persistence.save_facade(&index).unwrap();
    drop(documents);
    drop(index);
    let mut index = persistence.load_facade_lite(config).unwrap();
    fs::write(&caller, [0xff, 0xfe]).unwrap();
    assert!(index.index_directory(&scanned, false).is_err());
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .contains(&caller)
    );
    fs::write(&caller, source).unwrap();
    index.index_directory(&scanned, false).unwrap();
    assert_eq!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .len(),
        1
    );
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .is_empty()
    );
    assert_fresh(
        &index,
        &roots,
        &temp.path().join("fresh"),
        "outside-root recovery retry",
    );
}

#[test]
fn phase_one_replacement_recovers_captured_incoming_calls_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("a.py"), "def target():\n    return 1\n").unwrap();
    fs::write(
        src.join("b.py"),
        "from a import target\ndef caller():\n    return target()\n",
    )
    .unwrap();
    let config = settings(&temp.path().join("index"), std::slice::from_ref(&src));
    let persistence = IndexPersistence::new(config.index_path.clone());
    let mut index = IndexFacade::new(Arc::clone(&config)).unwrap();
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .len(),
        1
    );
    fs::write(src.join("a.py"), "def target():\n    return 2\n").unwrap();
    fs::File::options()
        .write(true)
        .open(src.join("a.py"))
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    let mut pending = codanna::indexing::pipeline::PendingResolution::default();
    index
        .index_directory_deferred(&src, false, &mut pending)
        .unwrap();
    assert!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .is_empty()
    );
    persistence.save_facade(&index).unwrap();
    drop(pending);
    drop(index);
    let mut index = persistence.load_facade_lite(config).unwrap();
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index
            .get_called_functions(symbol(&index, "caller").id)
            .len(),
        1
    );
    assert!(
        index
            .document_index()
            .get_pending_resolution_paths()
            .unwrap()
            .is_empty()
    );
    assert_fresh(
        &index,
        &[src],
        &temp.path().join("fresh"),
        "incoming restart",
    );
}

//! Proposed lifecycle regressions for bpstr/codanna at
//! 1968a6fc14c4d5c080ac91b5e74799a4b0aa0cc2.
//! Run as an integration test, e.g. tests/review_index_lifecycle.rs.
//! These assert desired behavior and are expected to expose current defects.
//! All data is local and semantic search is disabled.

use codanna::indexing::{facade::IndexFacade, pipeline::stages::DiscoverStage};
use codanna::{RelationKind, ScopeContext, Settings, Symbol};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn facade(index: &Path, roots: &[PathBuf]) -> IndexFacade {
    let mut settings = Settings {
        index_path: index.to_path_buf(),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    for root in roots {
        settings.add_indexed_path(root.clone()).unwrap();
    }
    IndexFacade::new(Arc::new(settings)).unwrap()
}

fn symbol(index: &IndexFacade, name: &str) -> Symbol {
    let matches = index.find_symbols_by_name(name, None);
    assert_eq!(matches.len(), 1, "fixture requires unique {name}");
    matches[0].clone()
}

fn scope(symbol: &Symbol) -> Option<String> {
    match &symbol.scope_context {
        Some(ScopeContext::ClassMember { class_name }) => {
            class_name.as_ref().map(ToString::to_string)
        }
        _ => None,
    }
}

fn pin_time(path: &Path, instant: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(instant)
        .unwrap();
}

#[test]
fn removing_alpha_member_does_not_rebind_its_caller_to_beta_member() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("types.rs"),
        "pub struct Alpha;\npub struct Beta;\nimpl Alpha { pub fn make() -> u32 { 1 } }\nimpl Beta { pub fn make() -> u32 { 2 } }\n").unwrap();
    fs::write(
        src.join("user.rs"),
        "use crate::types::Alpha;\npub fn go() -> u32 { Alpha::make() }\n",
    )
    .unwrap();
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    let before = index.get_called_functions(symbol(&index, "go").id);
    assert_eq!(before.len(), 1, "fixture must resolve Alpha::make");
    assert_eq!(scope(&before[0]).as_deref(), Some("Alpha"));

    fs::write(
        src.join("types.rs"),
        "pub struct Alpha;\npub struct Beta;\nimpl Beta { pub fn make() -> u32 { 2 } }\n",
    )
    .unwrap();
    index.index_file(src.join("types.rs")).unwrap();
    let after = index.get_called_functions(symbol(&index, "go").id);
    assert!(
        after.is_empty(),
        "Alpha::make was removed; must not rebind to surviving {:?}",
        after
            .iter()
            .map(|s| (s.name.to_string(), scope(s)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn adding_missing_import_target_repairs_unchanged_caller() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("caller.ts"),
        "import { sharedTarget } from './target';\nexport function entry() { return sharedTarget(); }\n").unwrap();
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    assert!(
        index
            .get_called_functions(symbol(&index, "entry").id)
            .is_empty()
    );
    fs::write(
        src.join("target.ts"),
        "export function sharedTarget() { return 42; }\n",
    )
    .unwrap();
    index.index_directory(&src, false).unwrap();

    let mut oracle = facade(&temp.path().join("fresh-index"), std::slice::from_ref(&src));
    oracle.index_directory(&src, false).unwrap();
    let fresh = oracle.get_called_functions(symbol(&oracle, "entry").id);
    assert_eq!(fresh.len(), 1, "fresh parse must resolve this fixture");
    let incremental = index.get_called_functions(symbol(&index, "entry").id);
    assert_eq!(
        incremental.len(),
        fresh.len(),
        "adding target.ts should repair caller.ts without touching caller.ts"
    );
}

#[test]
fn changing_reexport_retargets_unchanged_consumer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fixture");
    let pkg = root.join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("__init__.py"), "from pkg.a import helper\n").unwrap();
    fs::write(pkg.join("a.py"), "def helper(x):\n    return x + 1\n").unwrap();
    fs::write(pkg.join("b.py"), "def helper(x):\n    return x + 2\n").unwrap();
    fs::write(
        pkg.join("consumer.py"),
        "from pkg import helper\n\ndef reexport_caller(x):\n    return helper(x)\n",
    )
    .unwrap();
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&root));
    index
        .index_directory_with_options(&root, false, false, false, None)
        .unwrap();
    let before = index.get_called_functions(symbol(&index, "reexport_caller").id);
    assert_eq!(before.len(), 1);
    assert!(Path::new(before[0].file_path.as_ref()).ends_with("a.py"));

    fs::write(pkg.join("__init__.py"), "from pkg.b import helper\n").unwrap();
    index.index_file(pkg.join("__init__.py")).unwrap();
    let mut oracle = facade(
        &temp.path().join("fresh-index"),
        std::slice::from_ref(&root),
    );
    oracle
        .index_directory_with_options(&root, false, false, false, None)
        .unwrap();
    let fresh = oracle.get_called_functions(symbol(&oracle, "reexport_caller").id);
    assert_eq!(fresh.len(), 1);
    assert!(Path::new(fresh[0].file_path.as_ref()).ends_with("b.py"));
    let after = index.get_called_functions(symbol(&index, "reexport_caller").id);
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].file_path, fresh[0].file_path,
        "the barrel switched from a.py to b.py; the unchanged consumer must follow it"
    );
}

#[test]
fn multi_root_edits_never_resurrect_old_caller_ids() {
    for callee_first in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let tests = temp.path().join("tests");
        fs::create_dir_all(src.join("pkg")).unwrap();
        fs::create_dir(&tests).unwrap();
        fs::write(src.join("pkg/__init__.py"), "").unwrap();
        let target = src.join("pkg/mod.py");
        let caller = tests.join("test_mod.py");
        let target_text = "def target_function():\n    return 42\n";
        let caller_text = "from pkg.mod import target_function\n\ndef test_target_function():\n    assert target_function() == 42\n";
        fs::write(&target, target_text).unwrap();
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
        assert_eq!(
            index
                .get_called_functions(symbol(&index, "test_target_function").id)
                .len(),
            1
        );
        let initial_count = index.relationship_count();
        fs::write(&target, format!("# shifted\n{target_text}")).unwrap();
        fs::write(&caller, format!("{caller_text}\n# touched\n")).unwrap();
        let hot = SystemTime::now() + Duration::from_secs(5);
        pin_time(&target, hot);
        pin_time(&caller, hot);
        index
            .index_directories_with_options(&roots, false, false, false, None)
            .unwrap();
        let target_symbol = symbol(&index, "target_function");
        for (from, to, _) in index
            .get_relationships_for_symbol(target_symbol.id)
            .unwrap()
        {
            assert!(
                index.get_symbol(from).is_some(),
                "dead caller {from:?} resurrected; callee_first={callee_first}"
            );
            assert!(index.get_symbol(to).is_some(), "dead target {to:?}");
        }
        assert_eq!(
            index.relationship_count(),
            initial_count,
            "comment edits must preserve edge count; callee_first={callee_first}"
        );
    }
}

#[test]
fn delayed_rescan_detects_different_nanoseconds_in_same_mtime_second() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    let file = src.join("a.rs");
    let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    fs::write(&file, "pub fn old_name() {}\n").unwrap();
    pin_time(&file, base + Duration::from_millis(100));
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    assert_eq!(index.find_symbols_by_name("old_name", None).len(), 1);
    fs::write(&file, "pub fn new_name() {}\n").unwrap();
    pin_time(&file, base + Duration::from_millis(900));
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index.find_symbols_by_name("new_name", None).len(),
        1,
        "a real subsecond mtime change must be detected when rescan happens in a later second"
    );
    assert!(index.find_symbols_by_name("old_name", None).is_empty());
}

#[test]
fn batch_discovery_rejects_partial_invalid_ignore_rules() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("a.rs"), "pub fn visible() {}\n").unwrap();
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    fs::write(src.join(".codannaignore"), "[z-a]\na.rs\n").unwrap();
    let discovery = DiscoverStage::new(&src, 1).with_index(index.document_index().clone());
    assert!(
        discovery.run_incremental().is_err(),
        "an invalid rule attached to a valid walk entry must not become deletion evidence"
    );
    assert_eq!(index.find_symbols_by_name("visible", None).len(), 1);
}

#[test]
fn max_files_limits_real_indexing_as_well_as_preview() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    for name in ["one", "two", "three"] {
        fs::write(
            src.join(format!("{name}.rs")),
            format!("pub fn {name}() {{}}\n"),
        )
        .unwrap();
    }
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index
        .index_directory_with_options(&src, false, false, false, Some(1))
        .unwrap();
    assert_eq!(
        index.get_all_indexed_paths().len(),
        1,
        "max_files=1 must bound committed files, not just the progress-bar denominator"
    );
}

#[test]
fn deleting_then_restoring_target_recovers_unchanged_caller() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    let target = src.join("target.ts");
    fs::write(&target, "export function sharedTarget() { return 42; }\n").unwrap();
    fs::write(src.join("caller.ts"),
        "import { sharedTarget } from './target';\nexport function entry() { return sharedTarget(); }\n").unwrap();
    let mut index = facade(&temp.path().join("index"), std::slice::from_ref(&src));
    index.index_directory(&src, false).unwrap();
    assert_eq!(
        index.get_called_functions(symbol(&index, "entry").id).len(),
        1
    );
    fs::remove_file(&target).unwrap();
    index.index_directory(&src, false).unwrap();
    assert!(
        index
            .get_called_functions(symbol(&index, "entry").id)
            .is_empty()
    );
    // A separate reopening case can wrap this state in IndexPersistence save/load;
    // first assert the simpler same-process restoration contract.
    fs::write(&target, "export function sharedTarget() { return 42; }\n").unwrap();
    index.index_directory(&src, false).unwrap();
    let entry = symbol(&index, "entry");
    let calls = index
        .document_index()
        .get_relationships_from(entry.id, RelationKind::Calls)
        .unwrap();
    assert_eq!(
        calls.len(),
        1,
        "restoring a deleted module must restore unchanged caller evidence"
    );
}

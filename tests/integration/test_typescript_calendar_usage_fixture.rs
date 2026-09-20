use codanna::config::Settings;
use codanna::indexing::facade::IndexFacade;
use codanna::{RelationKind, SymbolKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FIXTURE_ROOT: &str = "tests/fixtures/typescript_calendar_usage";

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT)
}

fn facade_for(root: &Path, index_path: PathBuf) -> IndexFacade {
    let mut settings = Settings {
        index_path,
        workspace_root: Some(root.to_path_buf()),
        ..Default::default()
    };
    settings
        .add_indexed_path(root.to_path_buf())
        .expect("register fixture path");
    IndexFacade::new(Arc::new(settings)).expect("create fixture index")
}

fn assert_function(facade: &IndexFacade, name: &str) {
    let symbols = facade.find_symbols_by_name(name, None);
    assert_eq!(symbols.len(), 1, "expected one `{name}`, got {symbols:?}");
    assert_eq!(symbols[0].kind, SymbolKind::Function, "`{name}` kind");
}

fn inbound_uses(facade: &IndexFacade, target_name: &str) -> Vec<String> {
    let targets = facade.find_symbols_by_name(target_name, None);
    assert_eq!(
        targets.len(),
        1,
        "fixture expects exactly one `{target_name}`"
    );

    let mut callers: Vec<String> = facade
        .get_relationships_for_symbol(targets[0].id)
        .expect("read fixture relationships")
        .into_iter()
        .filter(|(_, to, relationship)| {
            *to == targets[0].id && relationship.kind == RelationKind::Uses
        })
        .filter_map(|(from, _, _)| facade.get_symbol(from))
        .map(|symbol| symbol.name.to_string())
        .collect();
    callers.sort();
    callers
}

#[test]
fn calendar_fixture_indexes_tsx_functions_and_component_usage() {
    let temp = tempfile::tempdir().expect("temp index directory");
    let root = fixture_path();
    let mut facade = facade_for(&root, temp.path().join("index"));

    facade
        .index_directory(&root, true)
        .expect("index calendar fixture");

    for name in [
        "Calendar",
        "TaskViewControls",
        "RangeCalendarPanel",
        "TimesheetPage",
        "WeekStrip",
    ] {
        assert_function(&facade, name);
    }

    assert_eq!(
        inbound_uses(&facade, "Calendar"),
        ["RangeCalendarPanel", "TimesheetPage"],
        "static imports rendered through JSX must produce Uses relationships"
    );
    assert_eq!(
        inbound_uses(&facade, "RangeCalendarPanel"),
        ["TaskViewControls"],
        "local JSX composition must produce a Uses relationship"
    );
}

#[test]
fn single_file_reindex_replaces_changed_tsx_symbols_and_relationships() {
    let temp = tempfile::tempdir().expect("temp fixture workspace");
    let root = temp.path().join("src");
    std::fs::create_dir_all(&root).expect("create fixture source directory");

    let source = fixture_path().join("week-strip.tsx");
    let changed = fixture_path().join("week-strip.updated.fixture");
    let indexed = root.join("week-strip.tsx");
    std::fs::copy(&source, &indexed).expect("copy initial TSX fixture");

    let mut facade = facade_for(&root, temp.path().join("index"));
    facade
        .index_directory(&root, true)
        .expect("index initial TSX fixture");
    assert_function(&facade, "WeekStrip");
    assert!(facade.find_symbols_by_name("WeekStripDay", None).is_empty());

    std::fs::copy(&changed, &indexed).expect("replace TSX fixture");
    facade
        .index_file(&indexed)
        .expect("reindex changed TSX file");

    assert_function(&facade, "WeekStrip");
    assert_function(&facade, "WeekStripDay");
    assert_eq!(
        inbound_uses(&facade, "WeekStripDay"),
        ["WeekStrip"],
        "single-file reindex must persist new TSX symbols and JSX relationships"
    );
}

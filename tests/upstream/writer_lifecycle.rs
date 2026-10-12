//! Released upstream Phase 2 writer-lifecycle regressions adapted to the fork.

use codanna::indexing::{facade::IndexFacade, pipeline::PendingResolution};
use codanna::{RelationKind, Settings, storage::DocumentIndex};
use std::sync::Arc;

#[test]
fn defines_only_resolution_releases_the_writer_after_publication() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().canonicalize().unwrap();
    let root = workspace.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("decl.py"),
        "class Widget:\n    def alpha(self):\n        pass\n",
    )
    .unwrap();

    let mut settings = Settings {
        index_path: workspace.join("index"),
        workspace_root: Some(workspace.clone()),
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let settings = Arc::new(settings);
    let mut facade = IndexFacade::new(Arc::clone(&settings)).unwrap();

    let mut pending = PendingResolution::default();
    facade
        .index_directory_deferred(&root, false, &mut pending)
        .unwrap();
    facade.resolve_deferred(&mut pending).unwrap();

    let classes = facade.find_symbols_by_name("Widget", None);
    assert_eq!(classes.len(), 1);
    let rows = facade
        .document_index()
        .get_relationships_from(classes[0].id, RelationKind::Defines)
        .unwrap();
    assert_eq!(rows.len(), 1, "the Defines edge must be durable");
    assert_eq!(facade.get_symbol(rows[0].1).unwrap().name.as_ref(), "alpha");

    let other = DocumentIndex::new(settings.index_path.join("tantivy"), &settings).unwrap();
    other
        .start_batch()
        .expect("resolve_deferred must return with no Phase 2 writer held");
    other.rollback_batch().unwrap();
}

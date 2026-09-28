//! F03: deferred JSX composition is a Uses edge with exact module identity.
//! Sources are synthetic and semantic search is disabled.

use codanna::indexing::facade::IndexFacade;
use codanna::{IndexPersistence, RelationKind, Settings, Symbol, SymbolId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FIXTURE_ROOT: &str = "tests/fixtures/retrieval_findings/f03";

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT)
}

fn facade(root: &Path, index_path: PathBuf) -> IndexFacade {
    let mut settings = Settings {
        index_path,
        workspace_root: Some(root.to_path_buf()),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.to_path_buf()).unwrap();
    IndexFacade::new(Arc::new(settings)).unwrap()
}

fn exact(index: &IndexFacade, path: &str, name: &str) -> Symbol {
    let matches: Vec<_> = index
        .find_symbols_by_name(name, None)
        .into_iter()
        .filter(|symbol| Path::new(symbol.file_path.as_ref()).ends_with(path))
        .collect();
    assert_eq!(matches.len(), 1, "expected one {path}:{name}, got {matches:?}");
    matches.into_iter().next().unwrap()
}

fn outgoing(index: &IndexFacade, from: SymbolId, kind: RelationKind) -> Vec<Symbol> {
    index
        .document_index()
        .get_relationships_from(from, kind)
        .unwrap()
        .into_iter()
        .filter_map(|(_, to, _)| index.get_symbol(to))
        .collect()
}

fn assert_graph(index: &IndexFacade) {
    let provider = exact(index, "provider.tsx", "Calendar");
    let reference = exact(index, "reference.tsx", "Calendar");
    for picker in [
        exact(index, "view.tsx", "Picker"),
        exact(index, "uncached.tsx", "UncachedPicker"),
    ] {
        let uses = outgoing(index, picker.id, RelationKind::Uses);
        assert_eq!(uses.iter().map(|s| s.id).collect::<Vec<_>>(), [provider.id]);
        assert!(outgoing(index, picker.id, RelationKind::Calls).is_empty());
        assert!(!uses.iter().any(|symbol| symbol.id == reference.id));
    }
    for (path, name) in [
        ("external.tsx", "ExternalPicker"),
        ("shadowed.tsx", "ShadowedPicker"),
        ("reassigned.tsx", "ReassignedPicker"),
        ("cache-reassigned.tsx", "CacheReassignedPicker"),
        ("computed.tsx", "ComputedPicker"),
    ] {
        let picker = exact(index, path, name);
        assert!(outgoing(index, picker.id, RelationKind::Uses).is_empty(), "{path}:{name}");
        assert!(outgoing(index, picker.id, RelationKind::Calls).is_empty(), "{path}:{name}");
    }
    let incoming: Vec<_> = index
        .document_index()
        .get_relationships_to(provider.id, RelationKind::Uses)
        .unwrap()
        .into_iter()
        .filter_map(|(from, _, _)| index.get_symbol(from))
        .map(|symbol| symbol.name.to_string())
        .collect();
    assert_eq!(incoming.len(), 2);
    assert!(incoming.contains(&"Picker".to_owned()));
    assert!(incoming.contains(&"UncachedPicker".to_owned()));
}

#[test]
fn lazy_member_projection_resolves_as_persisted_composition_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_path();
    let mut index = facade(&root, temp.path().join("index"));
    index.index_directory(&root, true).unwrap();
    assert_graph(&index);

    let settings = Arc::clone(index.settings());
    let persistence = IndexPersistence::new(settings.index_path.clone());
    persistence.save_facade(&index).unwrap();
    drop(index);
    let reopened = persistence.load_facade_lite(settings).unwrap();
    assert_graph(&reopened);
}

//! Production graph regressions beyond the original investigation corpus.
//! All fixtures run with semantic search disabled and use local source only.

use codanna::{RelationKind, ScopeContext, Settings, Symbol, indexing::facade::IndexFacade};
use std::{fs, path::Path, sync::Arc};

fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir(&src).unwrap();
    for (name, source) in files {
        let path = src.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let mut settings = Settings {
        index_path: temp.path().join("index"),
        workspace_root: None,
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(src.clone()).unwrap();
    let mut facade = IndexFacade::new(Arc::new(settings)).unwrap();
    facade.index_directory(&src, false).unwrap();
    (temp, facade)
}

fn symbol(facade: &IndexFacade, name: &str) -> Symbol {
    let matches = facade.find_symbols_by_name(name, None);
    assert_eq!(matches.len(), 1, "expected unique {name}: {matches:?}");
    matches[0].clone()
}

fn assert_a_dispatch(facade: &IndexFacade, caller: &str) {
    let methods: Vec<_> = facade
        .get_called_functions(symbol(facade, caller).id)
        .into_iter()
        .filter(|s| s.name.as_ref() == "run")
        .collect();
    assert_eq!(methods.len(), 1, "{caller}: {methods:?}");
    assert!(
        matches!(methods[0].scope_context.as_ref(), Some(ScopeContext::ClassMember { class_name: Some(name) }) if name.as_ref() == "A"),
        "{methods:?}"
    );
}

#[test]
fn python_incremental_c3_keeps_unchanged_imported_ancestor_identities() {
    let derived = "from .bases import X, Y\nclass D(X,Y):\n    def via_self(self): self.run()\n    def via_super(self): super().run()\ndef execute(value: D): value.run()\n";
    let (temp, mut facade) = project(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/bases.py",
            "class A:\n    def run(self): pass\nclass X(A): pass\nclass Y:\n    def run(self): pass\n",
        ),
        ("pkg/derived.py", derived),
        (
            "decoy.py",
            "class X:\n    def run(self): pass\nclass Y:\n    def run(self): pass\n",
        ),
    ]);
    for caller in ["via_self", "via_super", "execute"] {
        assert_a_dispatch(&facade, caller);
    }
    let path = temp.path().join("src/pkg/derived.py");
    fs::write(&path, format!("# edit only the consumer\n{derived}")).unwrap();
    facade.index_file(&path).unwrap();
    for caller in ["via_self", "via_super", "execute"] {
        assert_a_dispatch(&facade, caller);
    }
}

fn implementations(facade: &IndexFacade) -> Vec<String> {
    let mut names: Vec<_> = facade
        .get_implementations(symbol(facade, "Contract").id)
        .into_iter()
        .map(|s| s.name.to_string())
        .collect();
    names.sort();
    names
}

fn replace(facade: &mut IndexFacade, root: &Path, name: &str, source: &str) {
    let path = root.join("src").join(name);
    fs::write(&path, source).unwrap();
    facade.index_file(path).unwrap();
}

#[test]
fn go_interface_signature_edit_removes_stale_implementation_without_rebinding_it() {
    let (temp, mut facade) = project(&[
        ("contract.go", "package p\ntype Contract interface{ M() }\n"),
        (
            "worker.go",
            "package p\ntype Worker struct{}\nfunc(Worker) M(){}\n",
        ),
    ]);
    assert_eq!(implementations(&facade), ["Worker"]);
    replace(
        &mut facade,
        temp.path(),
        "contract.go",
        "package p\ntype Contract interface{ M(int) }\n",
    );
    assert!(implementations(&facade).is_empty());
    replace(
        &mut facade,
        temp.path(),
        "contract.go",
        "package p\ntype Contract interface{ M() }\n",
    );
    assert_eq!(implementations(&facade), ["Worker"]);
    facade
        .index_directory(&temp.path().join("src"), false)
        .unwrap();
    let rows = facade
        .document_index()
        .get_relationships_to(symbol(&facade, "Contract").id, RelationKind::Implements)
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "repeated indexing must not duplicate derived rows"
    );
}

#[test]
fn go_embedded_method_edit_recomputes_unchanged_wrapper_implementation() {
    let (temp, mut facade) = project(&[
        (
            "contract.go",
            "package p\ntype Contract interface{ M() }\ntype Wrapper struct{ Base }\n",
        ),
        (
            "base.go",
            "package p\ntype Base struct{}\nfunc(Base) M(){}\n",
        ),
    ]);
    assert_eq!(implementations(&facade), ["Base", "Wrapper"]);
    replace(
        &mut facade,
        temp.path(),
        "base.go",
        "package p\ntype Base struct{}\nfunc(Base) M(int){}\n",
    );
    assert!(implementations(&facade).is_empty());
}

#[test]
fn go_pointer_only_relationship_retains_receiver_evidence() {
    let (_temp, facade) = project(&[(
        "p.go",
        "package p\ntype Contract interface{ M() }; type Worker struct{};func(*Worker) M(){}\n",
    )]);
    let rows = facade
        .document_index()
        .get_relationships_to(symbol(&facade, "Contract").id, RelationKind::Implements)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .2
            .metadata
            .as_ref()
            .and_then(|m| m.receiver.as_deref()),
        Some("*Worker")
    );
}

#[test]
fn python_annotation_dependencies_do_not_create_eager_call_edges() {
    let (_temp, facade) = project(&[(
        "types.py",
        "from __future__ import annotations\ndef compute_type(): return int\ndef entry(value: compute_type()): pass\n",
    )]);
    assert!(
        facade
            .get_called_functions(symbol(&facade, "entry").id)
            .is_empty()
    );
    let uses = facade
        .document_index()
        .get_relationships_from(symbol(&facade, "entry").id, RelationKind::Uses)
        .unwrap();
    assert!(
        uses.iter()
            .any(|(_, to, _)| *to == symbol(&facade, "compute_type").id)
    );
}

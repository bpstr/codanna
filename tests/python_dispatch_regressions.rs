//! Full graph checks for Python dispatch paths that must share C3 and import identity.
//! Synthetic sources are parsed locally; semantic search and providers are disabled.

use codanna::indexing::facade::IndexFacade;
use codanna::{ScopeContext, Settings, Symbol};
use std::fs;
use std::sync::Arc;

fn index_fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    fs::create_dir(&root).unwrap();
    for (name, content) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Settings::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root, true).unwrap();
    (temp, index)
}

fn run_targets(index: &IndexFacade, caller: &str) -> Vec<Symbol> {
    let callers = index.find_symbols_by_name(caller, Some("python"));
    assert_eq!(
        callers.len(),
        1,
        "fixture must index unique caller {caller}"
    );
    index
        .get_called_functions(callers[0].id)
        .into_iter()
        .filter(|target| target.name.as_ref() == "run")
        .collect()
}

#[test]
fn self_dispatch_uses_the_same_c3_order_as_an_annotated_receiver() {
    let source = r#"
class A:
    def run(self):
        return "A"
class X(A):
    pass
class Y:
    def run(self):
        return "Y"
class D(X, Y):
    def via_self(self):
        return self.run()
"#;
    let (_temp, index) = index_fixture(&[("mro.py", source)]);
    let targets = run_targets(&index, "via_self");
    assert_eq!(
        targets.len(),
        1,
        "self dispatch must resolve exactly once: {targets:?}"
    );
    assert!(
        matches!(targets[0].scope_context.as_ref(),
        Some(ScopeContext::ClassMember { class_name: Some(name) }) if name.as_ref() == "A"),
        "a later direct base cannot outrank the first base's inherited method: {targets:?}"
    );
}

#[test]
fn unresolved_imported_parent_never_uses_an_unrelated_same_named_class() {
    let (_temp, index) = index_fixture(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/child.py",
            r#"
from .missing import Base
class Child(Base):
    def via_super(self):
        return super().run()
    def via_self(self):
        return self.run()
"#,
        ),
        (
            "decoy.py",
            "class Base:\n    def run(self):\n        return 'unrelated'\n",
        ),
    ]);
    assert_eq!(
        index.find_symbols_by_name("Base", Some("python")).len(),
        1,
        "fixture must contain the tempting same-name decoy"
    );
    for caller in ["via_super", "via_self"] {
        assert!(
            run_targets(&index, caller).is_empty(),
            "unresolved explicit parent import must block global-name fallback for {caller}"
        );
    }
}

#[test]
fn undefined_parent_never_uses_an_unimported_global_class() {
    let (_temp, index) = index_fixture(&[
        (
            "child.py",
            "class Child(Base):\n    def via_super(self):\n        return super().run()\n",
        ),
        (
            "decoy.py",
            "class Base:\n    def run(self):\n        return 'unrelated'\n",
        ),
    ]);
    assert_eq!(index.find_symbols_by_name("Base", Some("python")).len(), 1);
    assert!(
        run_targets(&index, "via_super").is_empty(),
        "Python does not implicitly import a same-named class from another module"
    );
}

#[test]
fn inconsistent_c3_order_does_not_choose_an_arbitrary_parent_method() {
    // Syntactically valid editor input, but Python cannot construct D's MRO.
    let source = r#"
class X:
    def run(self):
        return "X"
class Y:
    def run(self):
        return "Y"
class A(X, Y):
    pass
class B(Y, X):
    pass
class D(A, B):
    def via_super(self):
        return super().run()
    def via_self(self):
        return self.run()
def execute(value: D):
    return value.run()
"#;
    let (_temp, index) = index_fixture(&[("inconsistent.py", source)]);
    for caller in ["via_super", "via_self", "execute"] {
        assert!(
            run_targets(&index, caller).is_empty(),
            "an inconsistent hierarchy must not pick an arbitrary run method for {caller}"
        );
    }
}

//! Additional parser semantics checks with deterministic inline source.

use codanna::parsing::LanguageParser;
use codanna::parsing::go::GoParser;
use codanna::parsing::python::PythonParser;
use codanna::parsing::rust::RustParser;

#[test]
fn rust_factory_evidence_does_not_cross_lexical_modules_or_import_aliases() {
    let code = "mod helper { struct Client; struct Wrong; impl Client { fn make() -> Wrong { Wrong } } }\nuse external::Client; fn entry() { let value = Client::make(); }";
    let mut parser = RustParser::new().unwrap();
    assert!(
        !parser
            .find_variable_types(code)
            .iter()
            .any(|(name, _, _)| *name == "value")
    );
}

#[test]
fn rust_factory_evidence_fails_closed_for_duplicate_owner_declarations() {
    let code = "struct Client; struct Token; impl Client { fn make() -> Token { Token } }\nmod other { struct Client; fn entry() { let value = Client::make(); } }";
    let mut parser = RustParser::new().unwrap();
    assert!(
        !parser
            .find_variable_types(code)
            .iter()
            .any(|(name, _, _)| *name == "value")
    );
}

#[test]
fn python_postponed_annotation_calls_are_type_uses() {
    let code = "from __future__ import annotations\ndef compute_type(): return int\ndef f(x: compute_type()): pass\n";
    let mut parser = PythonParser::new().unwrap();
    assert!(
        !parser
            .find_calls(code)
            .iter()
            .any(|(_, to, _)| *to == "compute_type")
    );
    assert!(
        !parser
            .find_method_calls(code)
            .iter()
            .any(|call| call.method_name == "compute_type")
    );
    assert!(
        parser
            .find_uses(code)
            .iter()
            .any(|(from, to, _)| *from == "f" && *to == "compute_type")
    );
}

#[test]
fn python_parent_positions_preserve_source_order() {
    let code = "class D(X, Y): pass\n";
    let mut parser = PythonParser::new().unwrap();
    let bases = parser.find_extends(code);
    assert_eq!(
        bases.iter().map(|(_, to, _)| *to).collect::<Vec<_>>(),
        ["X", "Y"]
    );
    assert!(bases[0].2.start_column < bases[1].2.start_column);
}

#[test]
fn go_indexed_callable_values_do_not_call_their_container() {
    let code = "package p\nfunc handlers[T any](values ...int){}\nfunc run(handlers []func(...int), i int){ handlers[0](); handlers[i](); handlers[i](1); handlers[i](1,2) }\n";
    let mut parser = GoParser::new().unwrap();
    assert!(
        !parser
            .find_calls(code)
            .iter()
            .any(|(from, to, _)| *from == "run" && *to == "handlers")
    );
    assert!(
        !parser
            .find_method_calls(code)
            .iter()
            .any(|c| c.caller == "run" && c.method_name == "handlers")
    );
}

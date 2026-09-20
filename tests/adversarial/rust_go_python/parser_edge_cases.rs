//! Parser and language-behavior regression fixtures from the adversarial review.
//! Fixture bodies are self-contained; no provider transport or inference is used.

use codanna::parsing::go::{GoBehavior, GoParser};
use codanna::parsing::python::{PythonInheritanceResolver, PythonParser};
use codanna::parsing::rust::RustParser;
use codanna::parsing::{InheritanceResolver, LanguageBehavior, LanguageParser};

#[test]
fn go_one_type_argument_call_survives_conversion_shaped_ast() {
    let code = include_str!("fixtures/go_generic_calls.go");
    let mut parser = GoParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(from, to, _)| *from == "GenericCalls" && *to == "Identity"),
        "calls: {calls:?}"
    );
}

#[test]
fn go_one_type_argument_call_survives_index_shaped_ast() {
    let code = include_str!("fixtures/go_generic_calls.go");
    let mut parser = GoParser::new().unwrap();
    let calls = parser.find_calls(code);
    for target in ["Pair", "Empty"] {
        assert!(
            calls
                .iter()
                .any(|(from, to, _)| *from == "GenericCalls" && *to == target),
            "missing {target}; calls: {calls:?}"
        );
    }
}

#[test]
fn go_two_type_arguments_control_is_already_extracted() {
    let code = include_str!("fixtures/go_generic_calls.go");
    let mut parser = GoParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(from, to, _)| *from == "GenericCalls" && *to == "TwoTypes"),
        "calls: {calls:?}"
    );
}

#[test]
fn rust_turbofish_preserves_free_call() {
    let code = include_str!("fixtures/rust_turbofish.rs");
    let mut parser = RustParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(from, to, _)| *from == "main" && *to == "identity"),
        "calls: {calls:?}"
    );
}

#[test]
fn rust_turbofish_preserves_method_call_and_receiver() {
    let code = include_str!("fixtures/rust_turbofish.rs");
    let mut parser = RustParser::new().unwrap();
    let calls = parser.find_method_calls(code);
    assert!(
        calls.iter().any(|c| c.caller == "main"
            && c.method_name == "accept"
            && c.receiver.as_deref() == Some("worker")),
        "calls: {calls:?}"
    );
}

#[test]
fn rust_explicit_local_annotation_outranks_factory_owner() {
    let code = "fn run() { let value: Token = Client::make_token(); value.consume(); }";
    let mut parser = RustParser::new().unwrap();
    let bindings = parser.find_variable_types(code);
    assert!(
        bindings
            .iter()
            .any(|(name, ty, _)| *name == "value" && *ty == "Token"),
        "bindings: {bindings:?}"
    );
    assert!(
        !bindings
            .iter()
            .any(|(name, ty, _)| *name == "value" && *ty == "Client")
    );
}

#[test]
fn rust_factory_owner_is_not_proof_of_return_type() {
    let code = include_str!("fixtures/rust_factory.rs");
    let mut parser = RustParser::new().unwrap();
    let bindings = parser.find_variable_types(code);
    assert!(
        !bindings
            .iter()
            .any(|(name, ty, _)| *name == "value" && *ty == "Client"),
        "wrong concrete type evidence: {bindings:?}"
    );
}

#[test]
fn go_shadowed_package_name_is_an_instance_receiver() {
    let code = include_str!("fixtures/go_bindings.go");
    let mut parser = GoParser::new().unwrap();
    let calls = parser.find_method_calls(code);
    let call = calls
        .iter()
        .find(|c| c.caller == "shadowedPackage" && c.method_name == "Reset")
        .unwrap();
    assert!(
        !call.is_static,
        "parameter bytes shadows imported bytes: {call:?}"
    );
}

#[test]
fn go_every_name_in_grouped_parameter_gets_the_type() {
    let behavior = GoBehavior::new();
    let sig = "func run(first, second *Codec)";
    assert_eq!(
        behavior.extract_parameter_type(sig, "first"),
        Some("Codec".into())
    );
    assert_eq!(
        behavior.extract_parameter_type(sig, "second"),
        Some("Codec".into())
    );
}

#[test]
fn go_generic_composite_literal_provides_receiver_type() {
    let code = include_str!("fixtures/go_bindings.go");
    let mut parser = GoParser::new().unwrap();
    let bindings = parser.find_variable_types(code);
    assert!(
        bindings
            .iter()
            .any(|(name, ty, _)| *name == "box" && *ty == "Box"),
        "bindings: {bindings:?}"
    );
}

#[test]
fn go_inferred_var_and_short_declaration_have_equal_evidence() {
    let code = include_str!("fixtures/go_bindings.go");
    let mut parser = GoParser::new().unwrap();
    let bindings = parser.find_variable_types(code);
    assert!(
        bindings
            .iter()
            .any(|(name, ty, _)| *name == "codec" && *ty == "Codec"),
        "bindings: {bindings:?}"
    );
}

#[test]
fn go_package_initializer_has_a_caller() {
    let code = include_str!("fixtures/go_bindings.go");
    let mut parser = GoParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls.iter().any(|(_, to, _)| *to == "loadConfig"),
        "calls: {calls:?}"
    );
}

#[test]
fn python_annotated_attribute_does_not_rebind_bare_local() {
    let code = include_str!("fixtures/python_attribute_binding.py");
    let mut parser = PythonParser::new().unwrap();
    let bindings = parser.find_variable_types(code);
    assert!(
        bindings
            .iter()
            .any(|(name, ty, _)| *name == "worker" && *ty == "First")
    );
    assert!(
        !bindings
            .iter()
            .any(|(name, ty, _)| *name == "worker" && *ty == "Second"),
        "attribute polluted local bindings: {bindings:?}"
    );
}

#[test]
fn python_default_argument_call_belongs_to_definition_environment() {
    let code = include_str!("fixtures/python_definition_time.py");
    let mut parser = PythonParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls
            .iter()
            .any(|(from, to, _)| *from == "<module>" && *to == "bootstrap"),
        "calls: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(from, to, _)| *from == "handle" && *to == "bootstrap")
    );
}

#[test]
fn python_bare_decorator_application_is_recorded() {
    let code = include_str!("fixtures/python_definition_time.py");
    let mut parser = PythonParser::new().unwrap();
    let calls = parser.find_calls(code);
    assert!(
        calls.iter().any(|(_, to, _)| *to == "register"),
        "calls: {calls:?}"
    );
}

#[test]
fn python_inheritance_helper_uses_c3_not_depth_first_deduplication() {
    let mut resolver = PythonInheritanceResolver::new();
    resolver.add_class("Root".into(), vec![]);
    resolver.add_class("Left".into(), vec!["Root".into()]);
    resolver.add_class("Right".into(), vec!["Root".into()]);
    resolver.add_class("Diamond".into(), vec!["Left".into(), "Right".into()]);
    resolver.add_class_methods("Root".into(), vec!["run".into()]);
    resolver.add_class_methods("Right".into(), vec!["run".into()]);
    assert_eq!(
        resolver.resolve_method("Diamond", "run"),
        Some("Right".into())
    );
}

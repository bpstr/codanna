//! Source references that do not establish an invocation.

use crate::Range;
use tree_sitter::Node;

/// A declaration name used as a value, with its enclosing source owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub source_name: String,
    pub source_range: Range,
    pub target_name: String,
    pub range: Range,
    pub context: &'static str,
}

fn range(node: Node<'_>) -> Range {
    Range::new(
        node.start_position().row as u32,
        node.start_position().column as u32,
        node.end_position().row as u32,
        node.end_position().column as u32,
    )
}

fn text<'a>(node: Node<'_>, code: &'a str) -> Option<&'a str> {
    node.utf8_text(code.as_bytes()).ok()
}

fn callable(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
    )
}

fn owner<'a>(node: Node<'_>, code: &'a str) -> Option<(&'a str, Range)> {
    let mut parent = node.parent();
    while let Some(scope) = parent {
        if callable(scope) {
            if let Some(name) = scope.child_by_field_name("name") {
                return Some((text(name, code)?, range(scope)));
            }
            if let Some(declaration) = scope.parent()
                && declaration.kind() == "variable_declarator"
                && let Some(name) = declaration.child_by_field_name("name")
            {
                return Some((text(name, code)?, range(declaration)));
            }
            // An anonymous callback has no stable declaration owner. Do
            // not attribute its deferred references to the outer function.
            return None;
        }
        parent = scope.parent();
    }
    None
}

fn parameter_names(node: Node<'_>, code: &str, names: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            if let Some(name) = text(node, code) {
                names.push(name.to_owned());
            }
        }
        "type_annotation" | "type_identifier" => {}
        "required_parameter"
        | "optional_parameter"
        | "assignment_pattern"
        | "object_assignment_pattern" => {
            // Default expressions are values, not declarations of the names
            // appearing in them. Only the left-hand pattern binds a name.
            if let Some(pattern) = node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("left"))
                .or_else(|| node.child_by_field_name("name"))
            {
                parameter_names(pattern, code, names);
            }
        }
        "pair_pattern" => {
            // {key: local} binds local, including when key is computed.
            if let Some(pattern) = node.child_by_field_name("value") {
                parameter_names(pattern, code, names);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                parameter_names(child, code, names);
            }
        }
    }
}

fn contains_binding(pattern: Node<'_>, name: &str, code: &str) -> bool {
    let mut names = Vec::new();
    parameter_names(pattern, code, &mut names);
    names.iter().any(|binding| binding == name)
}

/// Only inspect declared header bindings, never the iterable or initializer
/// values. Called on ancestors so a sibling loop cannot shadow the reference.
fn loop_binds_name(scope: Node<'_>, name: &str, code: &str) -> bool {
    match scope.kind() {
        // Tree-sitter uses this node for in, of, and await-of loops.
        "for_in_statement" if scope.child_by_field_name("kind").is_some() => scope
            .child_by_field_name("left")
            .is_some_and(|pattern| contains_binding(pattern, name, code)),
        "for_statement" => {
            let Some(initializer) = scope.child_by_field_name("initializer") else {
                return false;
            };
            if !matches!(
                initializer.kind(),
                "lexical_declaration" | "variable_declaration"
            ) {
                return false;
            }
            let mut cursor = initializer.walk();
            initializer.named_children(&mut cursor).any(|binding| {
                binding
                    .child_by_field_name("name")
                    .is_some_and(|pattern| contains_binding(pattern, name, code))
            })
        }
        _ => false,
    }
}

pub(crate) fn has_unresolved_binding(mut node: Node<'_>, name: &str, code: &str) -> bool {
    while let Some(parent) = node.parent() {
        if loop_binds_name(parent, name, code) {
            return true;
        }
        if callable(parent) {
            let parameters = parent
                .child_by_field_name("parameters")
                .or_else(|| parent.child_by_field_name("parameter"));
            if parameters.is_some_and(|parameters| contains_binding(parameters, name, code)) {
                return true;
            }
        }
        if parent.kind() == "catch_clause"
            && parent
                .child_by_field_name("parameter")
                .is_some_and(|parameter| contains_binding(parameter, name, code))
        {
            return true;
        }
        // Destructuring does not yet provide a declaration identity for each
        // extracted value. Preserve its lexical shadow instead of falling
        // through to a same-named outer declaration. Only inspect declarations
        // in an enclosing scope; an unrelated sibling block cannot shadow us.
        if matches!(
            parent.kind(),
            "statement_block" | "program" | "for_statement"
        ) {
            let mut cursor = parent.walk();
            for declaration in parent.named_children(&mut cursor) {
                if !matches!(
                    declaration.kind(),
                    "lexical_declaration" | "variable_declaration"
                ) {
                    continue;
                }
                let mut declaration_cursor = declaration.walk();
                for binding in declaration.named_children(&mut declaration_cursor) {
                    if let Some(pattern) = binding.child_by_field_name("name")
                        && pattern.kind() != "identifier"
                        && contains_binding(pattern, name, code)
                    {
                        return true;
                    }
                }
            }
        }
        node = parent;
    }
    false
}

/// Extract bare identifiers passed as JS/TS arguments. A callback or route
/// handler is a reference here; whether the callee invokes it is unknown.
/// Member expressions, strings and anonymous closures need richer evidence
/// and deliberately do not acquire a guessed target from this pass.
pub fn argument_references(root: Node<'_>, code: &str) -> Vec<Reference> {
    let mut references = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.kind() == "call_expression"
            && let Some(arguments) = node.child_by_field_name("arguments")
            && let Some((source_name, source_range)) = owner(node, code)
        {
            let mut cursor = node.walk();
            for argument in arguments.named_children(&mut cursor) {
                if argument.kind() != "identifier" {
                    continue;
                }
                let Some(name) = text(argument, code) else {
                    continue;
                };
                if has_unresolved_binding(node, name, code) {
                    continue;
                }
                references.push(Reference {
                    source_name: source_name.to_owned(),
                    source_range,
                    target_name: name.to_owned(),
                    range: range(argument),
                    context: "argument_reference",
                });
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    references
}

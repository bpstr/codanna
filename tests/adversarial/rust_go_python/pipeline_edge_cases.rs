//! Production ResolveStage probes with symbols, calls, and bindings obtained
//! from the real parsers. No index store, embeddings, or provider calls.

use codanna::indexing::pipeline::ResolveStage;
use codanna::indexing::pipeline::types::{
    ResolutionContext, ResolvedRelationship, SymbolLookupCache, UnresolvedRelationship,
    VariableBinding,
};
use codanna::parsing::python::{PythonBehavior, PythonParser};
use codanna::parsing::resolution::{GenericResolutionContext, ScopeLevel};
use codanna::parsing::rust::{RustBehavior, RustParser};
use codanna::parsing::{LanguageBehavior, LanguageId, LanguageParser, ResolutionScope};
use codanna::relationship::RelationshipMetadata;
use codanna::symbol::ScopeContext;
use codanna::types::{FileId, Range, SymbolCounter};
use codanna::{RelationKind, Symbol, SymbolKind};
use std::collections::HashMap;
use std::sync::Arc;

fn contains(outer: Range, inner: Range) -> bool {
    outer.contains(inner.start_line, inner.start_column)
        && outer.contains(inner.end_line, inner.end_column)
}

fn resolve_fixture(language: &'static str, code: &str) -> (Vec<Symbol>, Vec<ResolvedRelationship>) {
    let (mut parser, behavior): (Box<dyn LanguageParser>, Arc<dyn LanguageBehavior>) =
        match language {
            "python" => (
                Box::new(PythonParser::new().unwrap()),
                Arc::new(PythonBehavior::new()),
            ),
            "rust" => (
                Box::new(RustParser::new().unwrap()),
                Arc::new(RustBehavior::new()),
            ),
            _ => unreachable!(),
        };
    let language_id = LanguageId::new(language);
    let file_id = FileId::new(1).unwrap();
    let mut symbols = parser.parse(code, file_id, &mut SymbolCounter::new());
    let cache = Arc::new(SymbolLookupCache::new());
    let mut scope = GenericResolutionContext::new(file_id);
    for symbol in &mut symbols {
        symbol.language_id = Some(language_id);
        behavior.configure_symbol(symbol, Some("fixture"));
        cache.insert(symbol.clone());
        scope.add_symbol(symbol.name.to_string(), symbol.id, ScopeLevel::Module);
    }
    let mut unresolved = Vec::new();
    for call in parser.find_method_calls(code) {
        if call.receiver.is_none() {
            continue;
        }
        let caller = symbols
            .iter()
            .filter(|s| s.name.as_ref() == call.caller.as_str() && contains(s.range, call.range))
            .min_by_key(|s| {
                (
                    s.range.end_line - s.range.start_line,
                    s.range.end_column.saturating_sub(s.range.start_column),
                )
            });
        let Some(caller) = caller else {
            continue;
        };
        let meta = RelationshipMetadata::new()
            .at_position(call.range.start_line, call.range.start_column)
            .with_receiver(call.receiver.unwrap())
            .static_call(call.is_static);
        unresolved.push(UnresolvedRelationship {
            from_id: Some(caller.id),
            from_name: caller.name.clone(),
            to_name: call.method_name.into(),
            file_id,
            kind: RelationKind::Calls,
            metadata: Some(meta),
            to_range: Some(call.range),
        });
    }
    for (child, parent, range) in parser.find_extends(code) {
        let child_symbol = symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Class && s.name.as_ref() == child)
            .unwrap();
        unresolved.push(UnresolvedRelationship {
            from_id: Some(child_symbol.id),
            from_name: child.into(),
            to_name: parent.into(),
            file_id,
            kind: RelationKind::Extends,
            metadata: None,
            to_range: Some(range),
        });
    }
    let variable_bindings = parser
        .find_variable_types(code)
        .into_iter()
        .map(|(name, ty, range)| VariableBinding {
            name: name.into(),
            type_name: ty.into(),
            range,
        })
        .collect();
    let context = ResolutionContext {
        file_id,
        language_id,
        imports: vec![],
        local_symbols: symbols.iter().map(|s| s.id).collect(),
        scope: Box::new(scope),
        unresolved_rels: unresolved,
        variable_bindings,
        this_barrier_spans: parser.find_this_barrier_spans(code),
    };
    let stage = ResolveStage::new(cache, HashMap::from([(language_id, behavior)]));
    let (batch, _) = stage.resolve(&context);
    (symbols, batch.relationships)
}

fn targets_for<'a>(
    symbols: &'a [Symbol],
    rels: &[ResolvedRelationship],
    caller_name: &str,
    method_name: &str,
) -> Vec<&'a Symbol> {
    rels.iter()
        .filter(|r| r.kind == RelationKind::Calls)
        .filter_map(|r| {
            let from = symbols.iter().find(|s| s.id == r.from_id)?;
            let to = symbols.iter().find(|s| s.id == r.to_id)?;
            (from.name.as_ref() == caller_name && to.name.as_ref() == method_name).then_some(to)
        })
        .collect()
}

fn class_of(symbol: &Symbol) -> Option<&str> {
    match symbol.scope_context.as_ref() {
        Some(ScopeContext::ClassMember {
            class_name: Some(name),
        }) => Some(name.as_ref()),
        _ => None,
    }
}

#[test]
fn rust_factory_receiver_must_target_return_type_not_associated_owner() {
    let (symbols, rels) = resolve_fixture("rust", include_str!("fixtures/rust_factory.rs"));
    for caller in ["inferred", "annotated"] {
        let targets = targets_for(&symbols, &rels, caller, "consume");
        assert!(
            !targets.iter().any(|s| class_of(s) == Some("Client")),
            "{caller} points to wrong Client::consume: {targets:?}"
        );
    }
    let targets = targets_for(&symbols, &rels, "annotated", "consume");
    assert!(
        targets.iter().any(|s| class_of(s) == Some("Token")),
        "explicit Token must be sufficient: {targets:?}"
    );
}

#[test]
fn python_field_annotation_cannot_overwrite_local_receiver_type() {
    let (symbols, rels) = resolve_fixture(
        "python",
        include_str!("fixtures/python_attribute_binding.py"),
    );
    let targets = targets_for(&symbols, &rels, "execute", "run");
    assert_eq!(targets.len(), 1, "targets: {targets:?}");
    assert_eq!(class_of(targets[0]), Some("First"), "targets: {targets:?}");
}

#[test]
fn python_receiver_resolution_follows_mro_instead_of_shortest_distance() {
    let (symbols, rels) = resolve_fixture("python", include_str!("fixtures/python_mro.py"));
    let targets = targets_for(&symbols, &rels, "execute", "run");
    assert_eq!(targets.len(), 1, "targets: {targets:?}");
    assert_eq!(class_of(targets[0]), Some("A"), "targets: {targets:?}");
}

#[test]
fn python_super_visits_inherited_method_of_first_base() {
    let (symbols, rels) = resolve_fixture("python", include_str!("fixtures/python_mro.py"));
    let targets = targets_for(&symbols, &rels, "via_super", "run");
    assert_eq!(targets.len(), 1, "targets: {targets:?}");
    assert_eq!(class_of(targets[0]), Some("A"), "targets: {targets:?}");
}

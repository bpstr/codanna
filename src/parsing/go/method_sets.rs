//! Go structural implementation facts derived from indexed declarations.
//! Type identity, complete signatures, pointer receivers, field shadowing, and
//! ambiguous promotion matter; method names alone do not prove implementation.

use crate::parsing::Import;
use crate::{FileId, Symbol, SymbolId, SymbolKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MethodName {
    name: String,
    private_package: Option<String>,
}

#[derive(Clone, Debug)]
struct Method {
    name: MethodName,
    signature: String,
    pointer: bool,
}

#[derive(Default)]
struct TypeFacts {
    interface: bool,
    complete: bool,
    methods: Vec<Method>,
    fields: HashSet<String>,
    embeds: Vec<(SymbolId, bool)>,
}

type MethodSet = HashMap<MethodName, String>;

pub(crate) struct GoMethodSets {
    facts: HashMap<SymbolId, TypeFacts>,
    names: HashMap<(String, String), Option<SymbolId>>,
    symbols: HashMap<SymbolId, Symbol>,
    imports: HashMap<FileId, Vec<Import>>,
    packages: HashMap<FileId, String>,
    import_packages: HashMap<String, Option<(String, String)>>,
}

impl GoMethodSets {
    pub fn new(symbols: Vec<Symbol>, imports: HashMap<FileId, Vec<Import>>) -> Self {
        let mut result = Self {
            facts: HashMap::new(),
            names: HashMap::new(),
            symbols: HashMap::new(),
            imports,
            packages: HashMap::new(),
            import_packages: HashMap::new(),
        };
        for symbol in &symbols {
            if symbol.kind != SymbolKind::Module
                || !symbol
                    .signature
                    .as_deref()
                    .is_some_and(|s| s.starts_with("package "))
            {
                continue;
            }
            let module = symbol.module_path.as_deref().unwrap_or("");
            let package = symbol.name.to_string();
            let identity = format!("{module}\0{package}");
            result.packages.insert(symbol.file_id, identity.clone());
            // External test packages cannot supply declarations to imports.
            if !symbol.file_path.ends_with("_test.go") {
                let entry = result
                    .import_packages
                    .entry(module.to_string())
                    .or_insert_with(|| Some((identity.clone(), package.clone())));
                if entry.as_ref().is_some_and(|(old, _)| old != &identity) {
                    *entry = None;
                }
            }
        }
        for symbol in &symbols {
            if matches!(
                symbol.kind,
                SymbolKind::Struct | SymbolKind::Interface | SymbolKind::TypeAlias
            ) {
                let key = (result.package(symbol), symbol.name.to_string());
                result
                    .names
                    .entry(key)
                    .and_modify(|v| *v = None)
                    .or_insert(Some(symbol.id));
                result.symbols.insert(symbol.id, symbol.clone());
            }
        }
        let types: Vec<_> = result.symbols.values().cloned().collect();
        for symbol in types {
            result.read_type(&symbol);
        }
        for symbol in &symbols {
            if symbol.kind == SymbolKind::Method {
                result.read_method(symbol);
            }
        }
        result
    }

    fn package(&self, symbol: &Symbol) -> String {
        self.packages
            .get(&symbol.file_id)
            .cloned()
            .unwrap_or_else(|| format!("unknown-file:{}", symbol.file_id.value()))
    }

    fn type_id(&self, text: &str, owner: &Symbol) -> Option<SymbolId> {
        let (package, name) = if let Some((alias, name)) = text.rsplit_once('.') {
            let imports = self.imports.get(&owner.file_id)?;
            let mut candidates = imports.iter().filter_map(|i| {
                let (identity, declared_name) = self.import_packages.get(&i.path)?.as_ref()?;
                (i.alias.as_deref().unwrap_or(declared_name) == alias).then_some(identity)
            });
            let package = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }
            (package.clone(), name)
        } else {
            (self.package(owner), text)
        };
        self.names
            .get(&(package, name.to_string()))
            .copied()
            .flatten()
    }

    fn parser() -> Option<Parser> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
        Some(parser)
    }

    fn find<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
        if node.kind() == kind {
            return Some(node);
        }
        node.named_children(&mut node.walk())
            .find_map(|child| Self::find(child, kind))
    }

    fn read_type(&mut self, symbol: &Symbol) {
        let mut facts = TypeFacts {
            interface: symbol.kind == SymbolKind::Interface,
            complete: true,
            ..Default::default()
        };
        if !self.packages.contains_key(&symbol.file_id) {
            facts.complete = false;
        }
        let Some(signature) = symbol.signature.as_deref() else {
            facts.complete = false;
            self.facts.insert(symbol.id, facts);
            return;
        };
        let source = format!("package indexed\ntype {signature}\n");
        let Some(tree) = Self::parser().and_then(|mut p| p.parse(&source, None)) else {
            return;
        };
        let Some(spec) = Self::find(tree.root_node(), "type_spec") else {
            facts.complete = false;
            self.facts.insert(symbol.id, facts);
            return;
        };
        if tree.root_node().has_error() || spec.child_by_field_name("type_parameters").is_some() {
            facts.complete = false;
        }
        if let Some(body) = spec.child_by_field_name("type") {
            let fields = if facts.interface {
                Some(body)
            } else {
                Self::find(body, "field_declaration_list")
            };
            if let Some(fields) = fields {
                for field in fields.named_children(&mut fields.walk()) {
                    if facts.interface && field.kind() == "method_elem" {
                        continue;
                    }
                    let mut cursor = field.walk();
                    let names: Vec<_> = field.children_by_field_name("name", &mut cursor).collect();
                    if !names.is_empty() {
                        for name in names {
                            facts.fields.insert(source[name.byte_range()].to_string());
                        }
                        continue;
                    }
                    let ty = if facts.interface {
                        field.named_child(0)
                    } else {
                        field.child_by_field_name("type")
                    };
                    let Some(ty) = ty else {
                        facts.complete = false;
                        continue;
                    };
                    // Type-set unions and constraints need full type-set semantics.
                    if !matches!(ty.kind(), "type_identifier" | "qualified_type") {
                        facts.complete = false;
                        continue;
                    }
                    let name = &source[ty.byte_range()];
                    let pointer = source[field.byte_range()].trim_start().starts_with('*');
                    if let Some(id) = self.type_id(name, symbol) {
                        facts.embeds.push((id, pointer));
                        if !facts.interface {
                            facts
                                .fields
                                .insert(name.rsplit('.').next().unwrap_or(name).to_string());
                        }
                    } else {
                        facts.complete = false;
                    }
                }
            } else {
                facts.complete = false;
            }
        } else {
            facts.complete = false;
        }
        self.facts.insert(symbol.id, facts);
    }

    fn read_method(&mut self, symbol: &Symbol) {
        let Some(crate::ScopeContext::ClassMember {
            class_name: Some(owner_name),
        }) = symbol.scope_context.as_ref()
        else {
            return;
        };
        let Some(owner_id) = self.type_id(owner_name, symbol) else {
            return;
        };
        let Some(signature) = symbol.signature.as_deref() else {
            if let Some(f) = self.facts.get_mut(&owner_id) {
                f.complete = false;
            }
            return;
        };
        let is_method = signature.trim_start().starts_with("func ")
            || signature.trim_start().starts_with("func(");
        let source = if is_method {
            format!("package indexed\n{signature} {{}}\n")
        } else {
            format!("package indexed\ntype I interface {{ {signature} }}\n")
        };
        let Some(tree) = Self::parser().and_then(|mut p| p.parse(&source, None)) else {
            return;
        };
        if tree.root_node().has_error() {
            if let Some(facts) = self.facts.get_mut(&owner_id) {
                facts.complete = false;
            }
            return;
        }
        let node = Self::find(
            tree.root_node(),
            if is_method {
                "method_declaration"
            } else {
                "method_elem"
            },
        );
        let Some(node) = node else {
            if let Some(f) = self.facts.get_mut(&owner_id) {
                f.complete = false;
            }
            return;
        };
        let shape = self.call_signature(node, &source, symbol);
        let Some(shape) = shape else {
            if let Some(f) = self.facts.get_mut(&owner_id) {
                f.complete = false;
            }
            return;
        };
        let name = symbol
            .name
            .rsplit('.')
            .next()
            .unwrap_or(&symbol.name)
            .to_string();
        let private_package = name
            .chars()
            .next()
            .filter(|c| !c.is_uppercase())
            .map(|_| self.package(symbol));
        let pointer = node
            .child_by_field_name("receiver")
            .is_some_and(|r| Self::find(r, "pointer_type").is_some());
        if let Some(facts) = self.facts.get_mut(&owner_id) {
            facts.methods.push(Method {
                name: MethodName {
                    name,
                    private_package,
                },
                signature: shape,
                pointer,
            });
        }
    }

    fn call_signature(&self, node: Node, source: &str, owner: &Symbol) -> Option<String> {
        let parameters =
            self.parameter_types(node.child_by_field_name("parameters")?, source, owner)?;
        let result = match node.child_by_field_name("result") {
            Some(n) if n.kind() == "parameter_list" => self.parameter_types(n, source, owner)?,
            Some(n) => vec![self.canonical_type(n, source, owner)?],
            None => Vec::new(),
        };
        Some(format!(
            "({})->({})",
            parameters.join(","),
            result.join(",")
        ))
    }

    fn parameter_types(&self, list: Node, source: &str, owner: &Symbol) -> Option<Vec<String>> {
        let mut result = Vec::new();
        for parameter in list.named_children(&mut list.walk()) {
            let ty = self.canonical_type(parameter.child_by_field_name("type")?, source, owner)?;
            let ty = if parameter.kind() == "variadic_parameter_declaration" {
                format!("...{ty}")
            } else {
                ty
            };
            let mut cursor = parameter.walk();
            let count = parameter
                .children_by_field_name("name", &mut cursor)
                .count()
                .max(1);
            result.extend(std::iter::repeat_n(ty, count));
        }
        Some(result)
    }

    fn canonical_type(&self, node: Node, source: &str, owner: &Symbol) -> Option<String> {
        let raw = &source[node.byte_range()];
        // Predeclared names can be shadowed by package declarations. Even an
        // ambiguous declaration must prevent falling back to a builtin.
        if node.kind() == "type_identifier"
            && self
                .names
                .contains_key(&(self.package(owner), raw.to_string()))
        {
            return self
                .type_id(raw, owner)
                .map(|id| format!("type:{}", id.value()));
        }
        match node.kind() {
            "type_identifier"
                if matches!(
                    raw,
                    "bool"
                        | "byte"
                        | "rune"
                        | "int"
                        | "int8"
                        | "int16"
                        | "int32"
                        | "int64"
                        | "uint"
                        | "uint8"
                        | "uint16"
                        | "uint32"
                        | "uint64"
                        | "uintptr"
                        | "float32"
                        | "float64"
                        | "complex64"
                        | "complex128"
                        | "string"
                        | "error"
                        | "any"
                ) =>
            {
                Some(
                    match raw {
                        "byte" => "uint8",
                        "rune" => "int32",
                        _ => raw,
                    }
                    .into(),
                )
            }
            "type_identifier" | "qualified_type" => self
                .type_id(raw, owner)
                .map(|id| format!("type:{}", id.value())),
            "pointer_type" => Some(format!(
                "*{}",
                self.canonical_type(node.named_child(0)?, source, owner)?
            )),
            "slice_type" => Some(format!(
                "[]{}",
                self.canonical_type(node.child_by_field_name("element")?, source, owner)?
            )),
            "array_type" => {
                // Same-spelled package constants may have different values.
                // Accept only numeric literals until constant evaluation exists.
                let length = node.child_by_field_name("length")?;
                if length.kind() != "int_literal" {
                    return None;
                }
                let literal = source[length.byte_range()].replace('_', "");
                let (digits, radix) = if literal.starts_with("0x") || literal.starts_with("0X") {
                    (&literal[2..], 16)
                } else if literal.starts_with("0o") || literal.starts_with("0O") {
                    (&literal[2..], 8)
                } else if literal.starts_with("0b") || literal.starts_with("0B") {
                    (&literal[2..], 2)
                } else if literal.len() > 1 && literal.starts_with('0') {
                    (&literal[1..], 8)
                } else {
                    (literal.as_str(), 10)
                };
                let length = u128::from_str_radix(digits, radix).ok()?;
                Some(format!(
                    "[{length}]{}",
                    self.canonical_type(node.child_by_field_name("element")?, source, owner)?
                ))
            }
            "map_type" => Some(format!(
                "map[{}]{}",
                self.canonical_type(node.child_by_field_name("key")?, source, owner)?,
                self.canonical_type(node.child_by_field_name("value")?, source, owner)?
            )),
            "channel_type" => {
                let ty = node.child_by_field_name("value")?;
                let prefix: String = source[node.start_byte()..ty.start_byte()]
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                Some(format!(
                    "{prefix}{}",
                    self.canonical_type(ty, source, owner)?
                ))
            }
            "function_type" => self
                .call_signature(node, source, owner)
                .map(|s| format!("func{s}")),
            "interface_type" if node.named_child_count() == 0 => Some("any".into()),
            _ => None,
        }
    }

    fn method_set(&self, id: SymbolId, pointer: bool) -> Option<MethodSet> {
        if self.facts.get(&id)?.interface {
            return self.interface_set(id, &mut HashSet::new());
        }
        let mut result = MethodSet::new();
        let mut blocked = HashSet::new();
        let mut frontier = vec![(id, pointer, HashSet::new())];
        for _ in 0..128 {
            if frontier.is_empty() {
                return Some(result);
            }
            if frontier.len() > 4096 {
                return None;
            }
            let mut next = Vec::new();
            let mut layer: HashMap<MethodName, Vec<String>> = HashMap::new();
            let mut layer_blocked = HashSet::new();
            let mut layer_fields = HashSet::new();
            for (ty, pointer, mut path) in frontier {
                if !path.insert(ty) {
                    return None;
                }
                let facts = self.facts.get(&ty)?;
                if !facts.complete {
                    return None;
                }
                for field in &facts.fields {
                    layer_blocked.insert(field.clone());
                    layer_fields.insert(field.clone());
                }
                if facts.interface {
                    for (name, signature) in self.interface_set(ty, &mut HashSet::new())? {
                        layer_blocked.insert(name.name.clone());
                        if !blocked.contains(&name.name) {
                            layer.entry(name).or_default().push(signature);
                        }
                    }
                    continue;
                }
                for method in &facts.methods {
                    layer_blocked.insert(method.name.name.clone());
                    if !blocked.contains(&method.name.name) && (!method.pointer || pointer) {
                        layer
                            .entry(method.name.clone())
                            .or_default()
                            .push(method.signature.clone());
                    }
                }
                for &(embed, embedded_pointer) in &facts.embeds {
                    next.push((embed, pointer || embedded_pointer, path.clone()));
                }
            }
            for (name, signatures) in layer {
                if signatures.len() == 1 && !layer_fields.contains(&name.name) {
                    result.insert(name, signatures[0].clone());
                }
            }
            blocked.extend(layer_blocked);
            frontier = next;
        }
        None
    }

    fn interface_set(&self, id: SymbolId, active: &mut HashSet<SymbolId>) -> Option<MethodSet> {
        if active.len() >= 128 || !active.insert(id) {
            return None;
        }
        let facts = self.facts.get(&id)?;
        if !facts.complete || !facts.interface {
            return None;
        }
        let mut result = MethodSet::new();
        let mut add = |name, signature| result.insert(name, signature);
        for method in &facts.methods {
            if add(method.name.clone(), method.signature.clone())
                .is_some_and(|old| old != method.signature)
            {
                return None;
            }
        }
        for &(base, _) in &facts.embeds {
            for (name, signature) in self.interface_set(base, active)? {
                if add(name, signature.clone()).is_some_and(|old| old != signature) {
                    return None;
                }
            }
        }
        active.remove(&id);
        Some(result)
    }

    /// One edge per declaration/interface pair. A true flag means only *T
    /// implements the interface; false means both T and *T implement it.
    pub fn implementations(&self) -> Vec<(SymbolId, SymbolId, bool)> {
        let interfaces: Vec<_> = self
            .facts
            .iter()
            .filter(|(_, f)| f.interface)
            .filter_map(|(&id, _)| self.method_set(id, false).map(|set| (id, set)))
            .collect();
        let mut result = Vec::new();
        for (&id, facts) in &self.facts {
            if facts.interface {
                continue;
            }
            let value = self.method_set(id, false);
            let pointer = self.method_set(id, true);
            for (interface, required) in &interfaces {
                let matches = |set: &Option<MethodSet>| {
                    set.as_ref().is_some_and(|set| {
                        required
                            .iter()
                            .all(|(name, sig)| set.get(name) == Some(sig))
                    })
                };
                if matches(&value) {
                    result.push((id, *interface, false));
                } else if matches(&pointer) {
                    result.push((id, *interface, true));
                }
            }
        }
        result.sort_by_key(|(id, interface, _)| (id.value(), interface.value()));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsing::{LanguageParser, go::GoParser};
    use crate::types::SymbolCounter;

    fn facts(files: &[(&str, &str)]) -> (GoMethodSets, HashMap<String, SymbolId>) {
        let mut parser = GoParser::new().unwrap();
        let mut counter = SymbolCounter::new();
        let mut symbols = Vec::new();
        let mut imports = HashMap::new();
        let mut ids = HashMap::new();
        for (i, (package, source)) in files.iter().enumerate() {
            let file_id = FileId::new((i + 1) as u32).unwrap();
            imports.insert(file_id, parser.find_imports(source, file_id));
            for mut symbol in parser.parse(source, file_id, &mut counter) {
                symbol.module_path = Some((*package).into());
                if matches!(
                    symbol.kind,
                    SymbolKind::Struct | SymbolKind::Interface | SymbolKind::TypeAlias
                ) {
                    ids.insert(format!("{package}.{}", symbol.name), symbol.id);
                }
                symbols.push(symbol);
            }
        }
        (GoMethodSets::new(symbols, imports), ids)
    }

    #[test]
    fn method_sets_distinguish_pointer_value_and_embedded_receivers() {
        let (sets, ids) = facts(&[(
            "p",
            "package p\ntype I interface{ Read([]byte) (int,error) }; type Socket struct{}; func(s *Socket) Read(b []byte)(int,error){return 0,nil}; type Value struct{ Socket }; type Pointer struct{ *Socket }; type Wrong struct{}; func(Wrong) Read(string)(int,error){return 0,nil}",
        )]);
        let edges = sets.implementations();
        for (name, pointer_only) in [("Socket", true), ("Value", true), ("Pointer", false)] {
            assert!(
                edges.contains(&(ids[&format!("p.{name}")], ids["p.I"], pointer_only)),
                "{name}: {edges:?}"
            );
        }
        assert!(
            !edges
                .iter()
                .any(|(id, target, _)| *id == ids["p.Wrong"] && *target == ids["p.I"])
        );
    }

    #[test]
    fn method_sets_preserve_ambiguity_and_field_shadowing() {
        let (sets, ids) = facts(&[(
            "p",
            "package p\ntype I interface{ M() }; type A struct{}; func(A) M(){}; type B struct{}; func(B) M(){}; type Ambiguous struct{A;B}; type Field struct{A;M int}; type Direct struct{A;B}; func(Direct) M(){}",
        )]);
        let edges = sets.implementations();
        assert!(edges.contains(&(ids["p.Direct"], ids["p.I"], false)));
        for name in ["Ambiguous", "Field"] {
            assert!(
                !edges.iter().any(
                    |(id, target, _)| *id == ids[&format!("p.{name}")] && *target == ids["p.I"]
                )
            );
        }
    }

    #[test]
    fn method_sets_compare_signatures_and_merge_interface_requirements() {
        let (sets, ids) = facts(&[(
            "p",
            "package p\ntype A interface{ M(a,b int) }; type B interface{ M(int,int) }; type Both interface{A;B}; type Good struct{};func(Good) M(first,second int){};type Bad struct{};func(Bad) M(int){};type Variadic interface{V(...int)};type Slice struct{};func(Slice) V([]int){}",
        )]);
        let edges = sets.implementations();
        assert!(edges.contains(&(ids["p.Good"], ids["p.Both"], false)));
        assert!(!edges.iter().any(|(id, target, _)| (*id == ids["p.Bad"]
            && *target == ids["p.Both"])
            || (*id == ids["p.Slice"] && *target == ids["p.Variadic"])));
    }

    #[test]
    fn method_sets_keep_private_names_and_shadowed_builtins_package_specific() {
        let (sets, ids) = facts(&[
            (
                "a",
                "package a\ntype Private interface{ hidden() }; type Bytes interface{ M(uint8) }; type error struct{}; type Errors interface{ E(error) }",
            ),
            (
                "b",
                "package b\ntype byte string; type T struct{};func(T) hidden(){};func(T) M(byte){};func(T) E(error){}",
            ),
        ]);
        assert!(
            !sets
                .implementations()
                .iter()
                .any(|(id, _, _)| *id == ids["b.T"])
        );
    }

    #[test]
    fn method_sets_do_not_equate_unresolved_array_constants() {
        let (sets, ids) = facts(&[
            (
                "a",
                "package a\nconst Width=8;type I interface{ M([Width]byte) };type Literal interface{L([16]byte)}",
            ),
            (
                "b",
                "package b\nconst Width=16;type T struct{};func(T) M([Width]byte){};type U struct{};func(U) L([0x10]byte){}",
            ),
        ]);
        let edges = sets.implementations();
        assert!(
            !edges
                .iter()
                .any(|(id, target, _)| *id == ids["b.T"] && *target == ids["a.I"])
        );
        assert!(edges.contains(&(ids["b.U"], ids["a.Literal"], false)));
    }

    #[test]
    fn method_sets_empty_interfaces_cover_all_complete_types_without_a_cap() {
        let mut source = "package p\ntype Empty interface{}\n".to_string();
        for i in 0..128 {
            source.push_str(&format!("type T{i} struct{{}}\n"));
        }
        let (sets, ids) = facts(&[("p", &source)]);
        let edges = sets.implementations();
        assert_eq!(
            edges
                .iter()
                .filter(|(_, to, _)| *to == ids["p.Empty"])
                .count(),
            128
        );
    }

    #[test]
    fn method_sets_external_test_package_cannot_satisfy_private_methods() {
        let (sets, ids) = facts(&[
            (
                "p",
                "package p\ntype Contract interface{ hidden() };type Internal struct{};func(Internal) hidden(){}",
            ),
            (
                "p",
                "package p_test\ntype External struct{};func(External) hidden(){}",
            ),
        ]);
        let edges = sets.implementations();
        assert!(edges.contains(&(ids["p.Internal"], ids["p.Contract"], false)));
        assert!(
            !edges
                .iter()
                .any(|(id, to, _)| *id == ids["p.External"] && *to == ids["p.Contract"])
        );
    }

    #[test]
    fn method_sets_external_test_package_named_types_have_distinct_identity() {
        let (sets, ids) = facts(&[
            (
                "p",
                "package p\ntype Payload struct{};type Contract interface{ M(Payload) }",
            ),
            (
                "p",
                "package p_test\ntype Payload struct{};type External struct{};func(External) M(Payload){}",
            ),
        ]);
        assert!(
            !sets
                .implementations()
                .iter()
                .any(|(id, to, _)| *id == ids["p.External"] && *to == ids["p.Contract"])
        );
    }
}

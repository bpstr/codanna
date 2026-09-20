//! Lightweight native tree-sitter support for Ruby and Bash.
//!
//! These parsers intentionally focus on the high-value structural surface used by
//! Codanna: symbols, calls, imports/sourcing, Ruby inheritance, and class-method
//! ownership. They do not execute interpreters, language servers, package managers,
//! or project code.

use crate::parsing::parser::{check_recursion_depth, truncate_for_display};
use crate::parsing::{
    HandledNode, Import, Language, LanguageBehavior, LanguageDefinition, LanguageId,
    LanguageParser, LanguageRegistry, NodeTracker, NodeTrackingState,
};
use crate::types::SymbolCounter;
use crate::{FileId, IndexError, IndexResult, Range, Settings, Symbol, SymbolKind, Visibility};
use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;
use tree_sitter::{Node, Parser, Tree};

const MODULE_SCOPE: &str = "<module>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Ruby,
    Bash,
}

impl Flavor {
    fn language(self) -> Language {
        match self {
            Self::Ruby => Language::Ruby,
            Self::Bash => Language::Bash,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
        }
    }
}

pub struct LightweightParser {
    parser: Parser,
    tree: Option<Tree>,
    flavor: Flavor,
    tracking: NodeTrackingState,
}

impl std::fmt::Debug for LightweightParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LightweightParser")
            .field("language", &self.flavor.language())
            .finish()
    }
}

impl LightweightParser {
    pub fn ruby() -> Result<Self, String> {
        Self::new(Flavor::Ruby)
    }

    pub fn bash() -> Result<Self, String> {
        Self::new(Flavor::Bash)
    }

    fn new(flavor: Flavor) -> Result<Self, String> {
        let mut parser = Parser::new();
        parser.set_language(&flavor.grammar()).map_err(|error| {
            format!("failed to initialize {} parser: {error}", flavor.language())
        })?;
        Ok(Self {
            parser,
            tree: None,
            flavor,
            tracking: NodeTrackingState::new(),
        })
    }

    fn range(node: Node<'_>) -> Range {
        Range::new(
            node.start_position().row as u32,
            node.start_position().column as u32,
            node.end_position().row as u32,
            node.end_position().column as u32,
        )
    }

    fn node_text<'a>(node: Node<'_>, code: &'a str) -> &'a str {
        &code[node.byte_range()]
    }

    fn name_node(node: Node<'_>) -> Option<Node<'_>> {
        node.child_by_field_name("name")
            .or_else(|| node.child_by_field_name("method"))
            .or_else(|| node.named_child(0))
    }

    fn symbol_shape(&self, node: Node<'_>) -> Option<SymbolKind> {
        match self.flavor {
            Flavor::Ruby => match node.kind() {
                "method" => Some(SymbolKind::Method),
                "singleton_method" => Some(SymbolKind::Method),
                "class" => Some(SymbolKind::Class),
                "module" => Some(SymbolKind::Module),
                _ => None,
            },
            Flavor::Bash => match node.kind() {
                "function_definition" => Some(SymbolKind::Function),
                _ => None,
            },
        }
    }

    fn extract_symbols(
        &mut self,
        node: Node<'_>,
        code: &str,
        file_id: FileId,
        counter: &mut SymbolCounter,
        symbols: &mut Vec<Symbol>,
        depth: usize,
    ) {
        if !check_recursion_depth(depth, node) {
            return;
        }
        self.register_handled_node(node.kind(), node.kind_id());

        if let Some(kind) = self.symbol_shape(node) {
            if let Some(name_node) = Self::name_node(node) {
                let name = Self::node_text(name_node, code).trim();
                if !name.is_empty() {
                    let mut symbol = Symbol::new(
                        counter.next_id(),
                        name.to_owned(),
                        kind,
                        file_id,
                        Self::range(node),
                    );
                    let source = Self::node_text(node, code);
                    let first_line = source.lines().next().unwrap_or(source).trim();
                    symbol.signature = Some(truncate_for_display(first_line, 320).into());
                    symbol.doc_comment = self.extract_doc_comment(&node, code).map(Into::into);
                    symbol.visibility = Visibility::Public;
                    symbols.push(symbol);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.extract_symbols(child, code, file_id, counter, symbols, depth + 1);
        }
    }

    fn boundary_name<'a>(&self, node: Node<'_>, code: &'a str) -> Option<&'a str> {
        let is_boundary = match self.flavor {
            Flavor::Ruby => matches!(node.kind(), "method" | "singleton_method"),
            Flavor::Bash => node.kind() == "function_definition",
        };
        if !is_boundary {
            return None;
        }
        Self::name_node(node).map(|name| Self::node_text(name, code).trim())
    }

    fn call_target<'a>(&self, node: Node<'_>, code: &'a str) -> Option<&'a str> {
        match self.flavor {
            Flavor::Ruby if node.kind() == "call" => node
                .child_by_field_name("method")
                .or_else(|| node.child_by_field_name("name"))
                .map(|name| Self::node_text(name, code).trim()),
            Flavor::Bash if node.kind() == "command" => node
                .child_by_field_name("name")
                .or_else(|| node.named_child(0))
                .map(|name| Self::node_text(name, code).trim()),
            _ => None,
        }
        .filter(|name| !name.is_empty())
    }

    fn walk_calls<'a>(
        &self,
        node: Node<'_>,
        code: &'a str,
        caller: &'a str,
        calls: &mut Vec<(&'a str, &'a str, Range)>,
        depth: usize,
    ) {
        if !check_recursion_depth(depth, node) {
            return;
        }
        let current = self.boundary_name(node, code).unwrap_or(caller);
        if let Some(target) = self.call_target(node, code) {
            // Ruby's require family and Bash's source/dot are dependency declarations,
            // not useful call-graph edges.
            let is_import = match self.flavor {
                Flavor::Ruby => matches!(target, "require" | "require_relative" | "load"),
                Flavor::Bash => matches!(target, "source" | "."),
            };
            if !is_import {
                calls.push((current, target, Self::range(node)));
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk_calls(child, code, current, calls, depth + 1);
        }
    }

    fn walk_extends<'a>(
        &self,
        node: Node<'_>,
        code: &'a str,
        result: &mut Vec<(&'a str, &'a str, Range)>,
        depth: usize,
    ) {
        if !check_recursion_depth(depth, node) {
            return;
        }
        if self.flavor == Flavor::Ruby && node.kind() == "class" {
            if let (Some(name), Some(parent)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("superclass"),
            ) {
                let derived = Self::node_text(name, code).trim();
                let base = Self::node_text(parent, code)
                    .trim()
                    .trim_start_matches('<')
                    .trim();
                if !derived.is_empty() && !base.is_empty() {
                    result.push((derived, base, Self::range(node)));
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk_extends(child, code, result, depth + 1);
        }
    }

    fn walk_defines<'a>(
        &self,
        node: Node<'_>,
        code: &'a str,
        owner: Option<&'a str>,
        result: &mut Vec<(&'a str, &'a str, Range)>,
        depth: usize,
    ) {
        if !check_recursion_depth(depth, node) {
            return;
        }
        if self.flavor != Flavor::Ruby {
            return;
        }
        let next_owner = if matches!(node.kind(), "class" | "module") {
            Self::name_node(node)
                .map(|n| Self::node_text(n, code).trim())
                .or(owner)
        } else {
            owner
        };
        if matches!(node.kind(), "method" | "singleton_method") {
            if let (Some(definer), Some(name)) = (next_owner, Self::name_node(node)) {
                result.push((
                    definer,
                    Self::node_text(name, code).trim(),
                    Self::range(node),
                ));
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk_defines(child, code, next_owner, result, depth + 1);
        }
    }

    fn import_from_node(&self, node: Node<'_>, code: &str, file_id: FileId) -> Option<Import> {
        let target = self.call_target(node, code)?;
        let accepted = match self.flavor {
            Flavor::Ruby => matches!(target, "require" | "require_relative" | "load"),
            Flavor::Bash => matches!(target, "source" | "."),
        };
        if !accepted {
            return None;
        }

        let node_source = Self::node_text(node, code);
        let rest = node_source.strip_prefix(target)?.trim_start();
        let rest = rest.trim_start_matches('(').trim_start();
        let quote = rest.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        let tail = &rest[quote.len_utf8()..];
        let end = tail.find(quote)?;
        let path = tail[..end].trim();
        if path.is_empty() {
            return None;
        }
        Some(Import {
            path: path.to_owned(),
            imported_name: None,
            alias: None,
            file_id,
            is_glob: false,
            is_type_only: false,
        })
    }

    fn walk_imports(
        &self,
        node: Node<'_>,
        code: &str,
        file_id: FileId,
        imports: &mut Vec<Import>,
        depth: usize,
    ) {
        if !check_recursion_depth(depth, node) {
            return;
        }
        if let Some(import) = self.import_from_node(node, code, file_id) {
            imports.push(import);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk_imports(child, code, file_id, imports, depth + 1);
        }
    }

    fn parse_tree(&mut self, code: &str) -> Option<Tree> {
        let tree = self.parser.parse(code, None)?;
        self.tree = Some(tree.clone());
        Some(tree)
    }
}

impl NodeTracker for LightweightParser {
    fn get_handled_nodes(&self) -> &HashSet<HandledNode> {
        self.tracking.get_handled_nodes()
    }

    fn register_handled_node(&mut self, node_kind: &str, node_id: u16) {
        self.tracking.register_handled_node(node_kind, node_id);
    }
}

impl LanguageParser for LightweightParser {
    fn parse(
        &mut self,
        code: &str,
        file_id: FileId,
        symbol_counter: &mut SymbolCounter,
    ) -> Vec<Symbol> {
        let Some(tree) = self.parse_tree(code) else {
            return Vec::new();
        };
        let mut symbols = Vec::new();
        self.extract_symbols(
            tree.root_node(),
            code,
            file_id,
            symbol_counter,
            &mut symbols,
            0,
        );
        symbols
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn extract_doc_comment(&self, node: &Node, code: &str) -> Option<String> {
        let row = node.start_position().row;
        if row == 0 {
            return None;
        }
        let lines: Vec<_> = code.lines().collect();
        let mut cursor = row;
        let mut docs = Vec::new();
        while cursor > 0 {
            cursor -= 1;
            let line = lines.get(cursor)?.trim();
            if line.is_empty() && docs.is_empty() {
                continue;
            }
            let Some(comment) = line.strip_prefix('#') else {
                break;
            };
            docs.push(comment.trim().to_owned());
        }
        docs.reverse();
        (!docs.is_empty()).then(|| docs.join("\n"))
    }

    fn find_calls<'a>(&mut self, code: &'a str) -> Vec<(&'a str, &'a str, Range)> {
        let Some(tree) = self.parse_tree(code) else {
            return Vec::new();
        };
        let mut calls = Vec::new();
        self.walk_calls(tree.root_node(), code, MODULE_SCOPE, &mut calls, 0);
        calls
    }

    fn find_implementations<'a>(&mut self, _code: &'a str) -> Vec<(&'a str, &'a str, Range)> {
        Vec::new()
    }

    fn find_extends<'a>(&mut self, code: &'a str) -> Vec<(&'a str, &'a str, Range)> {
        if self.flavor != Flavor::Ruby {
            return Vec::new();
        }
        let Some(tree) = self.parse_tree(code) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        self.walk_extends(tree.root_node(), code, &mut result, 0);
        result
    }

    fn find_uses<'a>(&mut self, _code: &'a str) -> Vec<(&'a str, &'a str, Range)> {
        Vec::new()
    }

    fn find_defines<'a>(&mut self, code: &'a str) -> Vec<(&'a str, &'a str, Range)> {
        if self.flavor != Flavor::Ruby {
            return Vec::new();
        }
        let Some(tree) = self.parse_tree(code) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        self.walk_defines(tree.root_node(), code, None, &mut result, 0);
        result
    }

    fn find_imports(&mut self, code: &str, file_id: FileId) -> Vec<Import> {
        let Some(tree) = self.parse_tree(code) else {
            return Vec::new();
        };
        let mut imports = Vec::new();
        self.walk_imports(tree.root_node(), code, file_id, &mut imports, 0);
        imports
    }

    fn language(&self) -> Language {
        self.flavor.language()
    }
}

pub struct LightweightBehavior {
    flavor: Flavor,
}

impl LightweightBehavior {
    pub fn ruby() -> Self {
        Self {
            flavor: Flavor::Ruby,
        }
    }

    pub fn bash() -> Self {
        Self {
            flavor: Flavor::Bash,
        }
    }
}

impl LanguageBehavior for LightweightBehavior {
    fn language_id(&self) -> LanguageId {
        match self.flavor {
            Flavor::Ruby => LanguageId::new("ruby"),
            Flavor::Bash => LanguageId::new("bash"),
        }
    }

    fn format_module_path(&self, base_path: &str, symbol_name: &str) -> String {
        if base_path.is_empty() {
            return symbol_name.to_owned();
        }
        format!("{base_path}{}{symbol_name}", self.module_separator())
    }

    fn parse_visibility(&self, _signature: &str) -> Visibility {
        Visibility::Public
    }

    fn module_separator(&self) -> &'static str {
        match self.flavor {
            Flavor::Ruby => "::",
            Flavor::Bash => "::",
        }
    }

    fn source_roots(&self) -> &'static [&'static str] {
        match self.flavor {
            Flavor::Ruby => &["app", "lib", "src"],
            Flavor::Bash => &["bin", "scripts", "script", "src"],
        }
    }

    fn format_path_as_module(&self, components: &[&str]) -> Option<String> {
        (!components.is_empty()).then(|| components.join(self.module_separator()))
    }

    fn supports_inherent_methods(&self) -> bool {
        self.flavor == Flavor::Ruby
    }

    fn get_language(&self) -> tree_sitter::Language {
        self.flavor.grammar()
    }
}

pub struct RubyLanguage;
pub struct BashLanguage;

impl LanguageDefinition for RubyLanguage {
    fn id(&self) -> LanguageId {
        LanguageId::new("ruby")
    }
    fn name(&self) -> &'static str {
        "Ruby"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["rb", "rake", "gemspec"]
    }
    fn create_parser(&self, _settings: &Settings) -> IndexResult<Box<dyn LanguageParser>> {
        Ok(Box::new(
            LightweightParser::ruby().map_err(IndexError::General)?,
        ))
    }
    fn create_behavior(&self) -> Box<dyn LanguageBehavior> {
        Box::new(LightweightBehavior::ruby())
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn is_enabled(&self, settings: &Settings) -> bool {
        settings
            .languages
            .get("ruby")
            .map(|c| c.enabled)
            .unwrap_or(true)
    }
}

impl LanguageDefinition for BashLanguage {
    fn id(&self) -> LanguageId {
        LanguageId::new("bash")
    }
    fn name(&self) -> &'static str {
        "Bash / POSIX shell"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["sh", "bash", "zsh", "bats"]
    }
    fn create_parser(&self, _settings: &Settings) -> IndexResult<Box<dyn LanguageParser>> {
        Ok(Box::new(
            LightweightParser::bash().map_err(IndexError::General)?,
        ))
    }
    fn create_behavior(&self) -> Box<dyn LanguageBehavior> {
        Box::new(LightweightBehavior::bash())
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn is_enabled(&self, settings: &Settings) -> bool {
        settings
            .languages
            .get("bash")
            .map(|c| c.enabled)
            .unwrap_or(true)
    }
}

pub(crate) fn register(registry: &mut LanguageRegistry) {
    registry.register(Arc::new(RubyLanguage));
    registry.register(Arc::new(BashLanguage));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_id() -> FileId {
        FileId::new(1).unwrap()
    }

    #[test]
    fn ruby_extracts_classes_methods_calls_imports_and_inheritance() {
        let code = r#"# API client
require 'json'
class Client < BaseClient
  # Fetch a record
  def fetch(id)
    JSON.parse(load_body(id))
  end
end
"#;
        let mut parser = LightweightParser::ruby().unwrap();
        let mut counter = SymbolCounter::new();
        let symbols = parser.parse(code, file_id(), &mut counter);
        assert!(
            symbols
                .iter()
                .any(|s| s.name.as_ref() == "Client" && s.kind == SymbolKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name.as_ref() == "fetch" && s.kind == SymbolKind::Method)
        );
        let calls = parser.find_calls(code);
        assert!(calls.iter().any(|(_, callee, _)| *callee == "parse"));
        assert!(calls.iter().any(|(_, callee, _)| *callee == "load_body"));
        let imports = parser.find_imports(code, file_id());
        assert!(imports.iter().any(|i| i.path == "json"));
        let extends = parser.find_extends(code);
        assert!(
            extends
                .iter()
                .any(|(child, parent, _)| *child == "Client" && *parent == "BaseClient")
        );
        let defines = parser.find_defines(code);
        assert!(
            defines
                .iter()
                .any(|(owner, method, _)| *owner == "Client" && *method == "fetch")
        );
    }

    #[test]
    fn bash_extracts_functions_calls_and_sources() {
        let code = r#"#!/usr/bin/env bash
source "./lib/common.sh"
build() {
  compile_assets
  echo done
}
build
"#;
        let mut parser = LightweightParser::bash().unwrap();
        let mut counter = SymbolCounter::new();
        let symbols = parser.parse(code, file_id(), &mut counter);
        assert!(
            symbols
                .iter()
                .any(|s| s.name.as_ref() == "build" && s.kind == SymbolKind::Function)
        );
        let calls = parser.find_calls(code);
        assert!(
            calls
                .iter()
                .any(|(caller, callee, _)| *caller == "build" && *callee == "compile_assets")
        );
        let imports = parser.find_imports(code, file_id());
        assert!(imports.iter().any(|i| i.path == "./lib/common.sh"));
    }

    #[test]
    fn definitions_register_extensions() {
        let mut registry = LanguageRegistry::new();
        register(&mut registry);
        assert_eq!(
            registry.get_by_extension("rb").unwrap().id(),
            LanguageId::new("ruby")
        );
        assert_eq!(
            registry.get_by_extension("sh").unwrap().id(),
            LanguageId::new("bash")
        );
    }
}

//! TypeScript-specific language behavior implementation

use crate::parsing::LanguageBehavior;
use crate::parsing::behavior_state::{BehaviorState, StatefulBehavior};
use crate::parsing::paths::strip_extension;
use crate::parsing::resolution::{InheritanceResolver, ResolutionScope};
use crate::project_resolver::persist::{ResolutionPersistence, ResolutionRules};
use crate::types::FileId;
use crate::{SymbolId, Visibility};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tree_sitter::{Language, Node};

/// AST descent: `required_parameter` | `optional_parameter` matched by pattern.
fn find_parameter_type(node: Node, code: &str, var_name: &str) -> Option<String> {
    if matches!(node.kind(), "required_parameter" | "optional_parameter") {
        let pattern = node.child_by_field_name("pattern")?;
        if pattern.kind() == "identifier" && &code[pattern.byte_range()] == var_name {
            let type_anno = node.child_by_field_name("type")?;
            return reduce_type_annotation(type_anno, code);
        }
    }
    for child in node.children(&mut node.walk()) {
        if let Some(found) = find_parameter_type(child, code, var_name) {
            return Some(found);
        }
    }
    None
}

/// Reduce TS `type_annotation` to bare name.
/// `T | null`, `T | undefined` -> inner `T`. `generic_type` -> base.
/// `nested_type_identifier` -> name field. object/function/tuple/intersection -> `None`.
/// Invariant: array/list methods do not live on inner `T` — do not strip generics.
fn reduce_type_annotation(node: Node, code: &str) -> Option<String> {
    let inner = if node.kind() == "type_annotation" {
        node.named_child(0)?
    } else {
        node
    };
    match inner.kind() {
        "type_identifier" | "predefined_type" => Some(code[inner.byte_range()].to_string()),
        "nested_type_identifier" => {
            let name = inner.child_by_field_name("name")?;
            Some(code[name.byte_range()].to_string())
        }
        "generic_type" => {
            let name = inner.child_by_field_name("name")?;
            reduce_type_annotation(name, code)
        }
        "union_type" => {
            let parts: Vec<_> = inner.named_children(&mut inner.walk()).collect();
            if parts.len() != 2 {
                return None;
            }
            let is_nullish = |n: Node| -> bool {
                if n.kind() != "literal_type" {
                    return false;
                }
                n.named_child(0)
                    .is_some_and(|c| matches!(c.kind(), "null" | "undefined"))
            };
            let non_null = if is_nullish(parts[1]) {
                parts[0]
            } else if is_nullish(parts[0]) {
                parts[1]
            } else {
                return None;
            };
            reduce_type_annotation(non_null, code)
        }
        _ => None,
    }
}

use super::resolution::{TypeScriptInheritanceResolver, TypeScriptResolutionContext};

type RulesCache = Option<(Instant, crate::project_resolver::persist::ResolutionIndex)>;

/// TypeScript language behavior implementation
#[derive(Clone)]
pub struct TypeScriptBehavior {
    state: BehaviorState,
    resolution_dir: PathBuf,
    rules_cache: Arc<Mutex<RulesCache>>,
}

impl TypeScriptBehavior {
    /// Create a behavior using this process's current workspace. Capture the
    /// directory once: subsequent CWD changes must not retarget an existing instance.
    pub fn new() -> Self {
        Self::with_resolution_dir(
            std::env::current_dir()
                .unwrap_or_default()
                .join(crate::init::local_dir_name()),
        )
    }

    /// Use an explicit `.codanna` directory, without modifying process CWD.
    /// Clones share this instance's cache; different workspaces never share rules.
    pub fn with_resolution_dir(resolution_dir: impl Into<PathBuf>) -> Self {
        Self {
            state: BehaviorState::new(),
            resolution_dir: resolution_dir.into(),
            rules_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Load only rules governing the registered file, never an arbitrary config.
    fn load_project_rules_for_file(&self, file_id: FileId) -> Option<ResolutionRules> {
        self.load_project_rules_for_path(&self.state.get_file_path(file_id)?)
    }

    fn load_project_rules_for_path(&self, path: &Path) -> Option<ResolutionRules> {
        let mut cache = self.rules_cache.lock().ok()?;
        if cache
            .as_ref()
            .is_none_or(|(timestamp, _)| timestamp.elapsed() >= Duration::from_secs(1))
        {
            // Invalidate first so an absent/broken replacement cannot resurrect
            // the previous workspace/config's rules after a failed refresh.
            *cache = None;
            let index = ResolutionPersistence::new(&self.resolution_dir)
                .load("typescript")
                .ok()?;
            *cache = Some((Instant::now(), index));
        }
        let (_, index) = cache.as_ref()?;
        index.rules.get(index.get_config_for_file(path)?).cloned()
    }
}

impl Default for TypeScriptBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl StatefulBehavior for TypeScriptBehavior {
    fn state(&self) -> &BehaviorState {
        &self.state
    }
}

impl LanguageBehavior for TypeScriptBehavior {
    fn language_id(&self) -> crate::parsing::registry::LanguageId {
        crate::parsing::registry::LanguageId::new("typescript")
    }

    fn configure_symbol(&self, symbol: &mut crate::Symbol, module_path: Option<&str>) {
        // Preserve parser-derived visibility (export detection), only set module path.
        if let Some(path) = module_path {
            let full_path = self.format_module_path(path, &symbol.name);
            symbol.module_path = Some(full_path.into());
        }
    }

    fn format_module_path(&self, base_path: &str, _symbol_name: &str) -> String {
        // TypeScript uses file paths as module paths, not including the symbol name
        // All symbols in the same file share the same module path for visibility
        base_path.to_string()
    }

    fn self_receiver_aliases(&self) -> &'static [&'static str] {
        &["this"]
    }

    fn self_alias_receiver_is_explicit(&self) -> bool {
        true
    }

    fn extract_parameter_type(&self, signature: &str, var_name: &str) -> Option<String> {
        let wrapped = format!("class __W__ {{ {signature} {{}} }}");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&self.get_language()).ok()?;
        let tree = parser.parse(&wrapped, None)?;
        find_parameter_type(tree.root_node(), &wrapped, var_name)
    }

    fn get_language(&self) -> Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
    fn module_separator(&self) -> &'static str {
        "."
    }

    fn module_path_from_file(
        &self,
        file_path: &Path,
        project_root: &Path,
        extensions: &[&str],
    ) -> Option<String> {
        // Use tsconfig infrastructure to compute canonical module paths
        // This ensures symbols use the SAME path format as enhanced imports

        // Load the resolution index to find which tsconfig governs this file
        let persistence = ResolutionPersistence::new(&self.resolution_dir);
        let index = persistence.load("typescript").ok()?;

        // get_config_for_file() canonicalizes its input; pass the absolute
        // path so the lookup matches whether the mapping globs were persisted
        // absolute (config entries outside the workspace) or
        // workspace-relative. A path stripped to workspace-relative fails
        // against absolute globs and silently nulls every module path.
        let config_path = index.get_config_for_file(file_path)?;
        tracing::debug!(
            "[typescript] module_path_from_file file_path={file_path:?} config_path={config_path:?}"
        );

        // Get the tsconfig's directory (the project root for this file)
        let tsconfig_dir = project_root.join(config_path.parent()?);
        tracing::debug!("[typescript] module_path_from_file tsconfig_dir={tsconfig_dir:?}");

        // Compute segments relative to the tsconfig's directory
        let mut segments = crate::parsing::paths::relative_segments(file_path, &tsconfig_dir)?;
        let last = segments.pop()?;
        let stem = strip_extension(&last, extensions);
        segments.push(stem.to_string());

        // index collapses to its directory (directory imports)
        while segments.len() > 1 && segments.last().is_some_and(|s| s == "index") {
            segments.pop();
        }
        let result = segments.join(".");

        tracing::debug!(
            "[typescript] module_path_from_file file_path={file_path:?} -> module_path={result}"
        );

        Some(result)
    }

    fn parse_visibility(&self, signature: &str) -> Visibility {
        // TypeScript visibility modifiers
        if signature.contains("export ") || signature.contains("export default") {
            Visibility::Public
        } else if signature.contains("private ") || signature.contains("#") {
            Visibility::Private
        } else if signature.contains("protected ") {
            // TypeScript has protected but Rust's Visibility enum doesn't
            // Map protected to Module visibility as a reasonable approximation
            Visibility::Module
        } else {
            // Default visibility for TypeScript symbols
            // Module-level symbols are private by default unless exported
            Visibility::Private
        }
    }

    fn supports_traits(&self) -> bool {
        true // TypeScript has interfaces
    }

    fn supports_inherent_methods(&self) -> bool {
        true // TypeScript has class methods
    }

    fn format_path_as_module(&self, components: &[&str]) -> Option<String> {
        if components.is_empty() {
            None
        } else {
            Some(components.join("."))
        }
    }

    // TypeScript-specific resolution overrides

    fn create_resolution_context(&self, file_id: FileId) -> Box<dyn ResolutionScope> {
        Box::new(TypeScriptResolutionContext::new(file_id))
    }

    fn create_inheritance_resolver(&self) -> Box<dyn InheritanceResolver> {
        Box::new(TypeScriptInheritanceResolver::new())
    }

    fn inheritance_relation_name(&self) -> &'static str {
        // TypeScript uses both "extends" and "implements"
        // Default to "extends" as it's more general
        "extends"
    }

    fn map_relationship(&self, language_specific: &str) -> crate::relationship::RelationKind {
        use crate::relationship::RelationKind;

        match language_specific {
            "extends" => RelationKind::Extends,
            "implements" => RelationKind::Implements,
            "uses" => RelationKind::Uses,
            "calls" => RelationKind::Calls,
            "defines" => RelationKind::Defines,
            _ => RelationKind::References,
        }
    }

    // Override import tracking methods to use state

    fn register_file(&self, path: PathBuf, file_id: FileId, module_path: String) {
        self.register_file_with_state(path, file_id, module_path);
    }

    fn add_import(&self, import: crate::parsing::Import) {
        // Store the ORIGINAL import path (including path aliases like @/components)
        // Enhancement will happen on-demand during resolution in build_resolution_context_with_cache()
        // This preserves the semantic information about whether a path was a tsconfig alias or a relative import
        tracing::debug!(
            "[typescript] add_import path='{}' alias={:?} file_id={:?}",
            import.path,
            import.alias,
            import.file_id
        );
        self.add_import_with_state(import);
    }

    fn get_imports_for_file(&self, file_id: FileId) -> Vec<crate::parsing::Import> {
        self.get_imports_from_state(file_id)
    }

    /// Build resolution context for parallel pipeline (no Tantivy).
    ///
    /// Uses tsconfig path aliases via TypeScriptProjectEnhancer.
    /// Returns (scope, enhanced_imports) where enhanced_imports have path aliases resolved.
    fn build_resolution_context_with_pipeline_cache(
        &self,
        file_id: FileId,
        imports: &[crate::parsing::Import],
        cache: &dyn crate::parsing::PipelineSymbolCache,
        extensions: &[&str],
    ) -> (
        Box<dyn crate::parsing::ResolutionScope>,
        Vec<crate::parsing::Import>,
    ) {
        use crate::parsing::ScopeLevel;
        use crate::parsing::resolution::{ImportBinding, ImportOrigin, ProjectResolutionEnhancer};

        // Helper to normalize relative imports
        fn normalize_import(import_path: &str, importing_mod: &str) -> String {
            if import_path.starts_with("./") {
                let rel = import_path.trim_start_matches("./").replace('/', ".");
                if importing_mod.is_empty() {
                    rel
                } else {
                    // Get parent of importing module
                    let parts: Vec<&str> = importing_mod.split('.').collect();
                    let parent = parts[..parts.len().saturating_sub(1)].join(".");
                    if parent.is_empty() {
                        rel
                    } else {
                        format!("{parent}.{rel}")
                    }
                }
            } else if import_path.starts_with("../") {
                // Just use the path as-is for now
                import_path.replace('/', ".")
            } else {
                // External or absolute
                import_path.replace('/', ".")
            }
        }

        let mut context = TypeScriptResolutionContext::new(file_id);

        let importing_module = self.get_module_path_for_file(file_id);
        let importing_file = cache
            .symbols_in_file(file_id)
            .first()
            .and_then(|id| cache.get(*id))
            .map(|sym| sym.file_path.to_string());

        // Load project rules for path alias enhancement
        let maybe_enhancer = self
            .load_project_rules_for_file(file_id)
            .or_else(|| {
                importing_file
                    .as_deref()
                    .and_then(|path| self.load_project_rules_for_path(Path::new(path)))
            })
            .map(super::resolution::TypeScriptProjectEnhancer::new);

        // Build enhanced imports with path aliases resolved
        let mut enhanced_imports = Vec::with_capacity(imports.len());

        for import in imports {
            // Get the local name to bind (alias or last path segment)
            let local_name = import.alias.clone().unwrap_or_else(|| {
                import
                    .path
                    .split('/')
                    .next_back()
                    .or_else(|| import.path.split('.').next_back())
                    .unwrap_or(&import.path)
                    .to_string()
            });
            let target_name = import.imported_name.as_deref().unwrap_or(&local_name);

            // Path-domain arm first: relative specifiers resolve by file
            // identity (trait default). Module-string normalization cannot
            // represent the navigation when stems contain dots.
            use crate::parsing::ExportResolution;
            let mut export_resolution =
                cache.resolve_export(file_id, &import.path, target_name, extensions);
            let file_resolved = match export_resolution {
                ExportResolution::Found(id) | ExportResolution::TypeOnly(id) => Some(id),
                ExportResolution::Unknown => importing_file.as_deref().and_then(|f| {
                    self.resolve_relative_import(cache, target_name, &import.path, f, extensions)
                }),
                ExportResolution::Missing | ExportResolution::Ambiguous => None,
            };
            let file_resolved_module = file_resolved
                .and_then(|id| cache.get(id))
                .and_then(|s| s.module_path.map(String::from));

            // Enhance import path if we have tsconfig rules
            let target_module = if let Some(module) = file_resolved_module {
                // The resolved file's parse-derived module is the truth the
                // string normalization approximates.
                module
            } else if let Some(ref enhancer) = maybe_enhancer {
                if let Some(enhanced_path) = enhancer.enhance_import_path(&import.path, file_id) {
                    // Tsconfig alias - convert enhanced path to module format
                    enhanced_path.trim_start_matches("./").replace('/', ".")
                } else {
                    // Regular import - normalize relative to importing module
                    normalize_import(&import.path, &importing_module.clone().unwrap_or_default())
                }
            } else {
                normalize_import(&import.path, &importing_module.clone().unwrap_or_default())
            };

            // Collect enhanced import with resolved path
            enhanced_imports.push(crate::parsing::Import {
                path: target_module.clone(),
                file_id: import.file_id,
                imported_name: import.imported_name.clone(),
                alias: import.alias.clone(),
                is_glob: import.is_glob,
                is_type_only: import.is_type_only,
            });

            // Look up candidates by local_name and match module_path. Exact
            // match wins outright; segment-boundary suffix matches bind only
            // an exactly-one survivor (candidate order is file-processing
            // order, not identity; raw ends_with also admitted mid-segment
            // captures).
            let mut resolved_symbol: Option<SymbolId> = file_resolved;
            if matches!(export_resolution, ExportResolution::Unknown) {
                export_resolution =
                    cache.resolve_module_export(&target_module, target_name, extensions);
                match export_resolution {
                    ExportResolution::Found(id) | ExportResolution::TypeOnly(id) => {
                        resolved_symbol = Some(id)
                    }
                    ExportResolution::Missing | ExportResolution::Ambiguous => {
                        resolved_symbol = None
                    }
                    ExportResolution::Unknown => {}
                }
            }
            let mut suffix_matches: Vec<SymbolId> = Vec::new();
            if resolved_symbol.is_none() && matches!(export_resolution, ExportResolution::Unknown) {
                for id in cache.lookup_candidates(target_name) {
                    if let Some(symbol) = cache.get(id) {
                        if let Some(ref module_path) = symbol.module_path {
                            if module_path.as_ref() == target_module {
                                resolved_symbol = Some(id);
                                break;
                            }
                            if crate::indexing::pipeline::types::segment_suffix_match(
                                module_path,
                                &target_module,
                            ) {
                                suffix_matches.push(id);
                            }
                        }
                    }
                }
                if resolved_symbol.is_none() {
                    if let [id] = suffix_matches.as_slice() {
                        resolved_symbol = Some(*id);
                    }
                }
            }

            // Determine origin
            let origin = if resolved_symbol.is_some()
                || matches!(
                    export_resolution,
                    ExportResolution::Missing | ExportResolution::Ambiguous
                ) {
                ImportOrigin::Internal
            } else {
                ImportOrigin::External
            };

            // Keep type-only export paths available for Uses while preventing
            // runtime Calls from treating them as ordinary value imports.
            let mut effective_import = import.clone();
            effective_import.is_type_only |=
                matches!(export_resolution, ExportResolution::TypeOnly(_));
            if let Some(enhanced) = enhanced_imports.last_mut() {
                enhanced.is_type_only = effective_import.is_type_only;
            }
            context.register_import_binding(ImportBinding {
                import: effective_import,
                exposed_name: local_name.clone(),
                origin,
                resolved_symbol,
            });

            if let (ImportOrigin::Internal, Some(symbol_id)) = (origin, resolved_symbol) {
                context.add_symbol(local_name.clone(), symbol_id, ScopeLevel::Module);
            }
        }

        // Populate context with enhanced imports
        context.populate_imports(&enhanced_imports);

        // Add local symbols from this file
        for sym_id in cache.symbols_in_file(file_id) {
            if let Some(symbol) = cache.get(sym_id) {
                if self.is_resolvable_symbol(&symbol) {
                    context.add_symbol(symbol.name.to_string(), symbol.id, ScopeLevel::Module);
                    if let Some(ref module_path) = symbol.module_path {
                        context.add_symbol(module_path.to_string(), symbol.id, ScopeLevel::Module);
                    }
                }
            }
        }

        (Box::new(context), enhanced_imports)
    }

    // TypeScript-specific: Support hoisting
    fn is_resolvable_symbol(&self, symbol: &crate::Symbol) -> bool {
        use crate::SymbolKind;
        use crate::symbol::ScopeContext;

        // TypeScript hoists function declarations and class declarations
        // They can be used before their definition in the file
        let hoisted = matches!(
            symbol.kind,
            SymbolKind::Function | SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
        );

        if hoisted {
            return true;
        }

        // Methods are always resolvable within their file
        if matches!(symbol.kind, SymbolKind::Method) {
            return true;
        }

        // Check scope_context for non-hoisted symbols
        if let Some(ref scope_context) = symbol.scope_context {
            match scope_context {
                ScopeContext::Module | ScopeContext::Global | ScopeContext::Package => true,
                ScopeContext::Local { .. } | ScopeContext::Parameter => false,
                ScopeContext::ClassMember { .. } => {
                    // Class members are resolvable if public or exported
                    matches!(symbol.visibility, Visibility::Public)
                }
            }
        } else {
            // Fallback for symbols without scope_context
            matches!(
                symbol.kind,
                SymbolKind::TypeAlias | SymbolKind::Constant | SymbolKind::Variable
            )
        }
    }

    fn get_module_path_for_file(&self, file_id: FileId) -> Option<String> {
        // Use the BehaviorState to get module path (O(1) lookup)
        self.state.get_module_path(file_id)
    }

    fn get_file_path(&self, file_id: FileId) -> Option<PathBuf> {
        self.state.get_file_path(file_id)
    }

    fn import_matches_symbol(
        &self,
        import_path: &str,
        symbol_module_path: &str,
        importing_module: Option<&str>,
    ) -> bool {
        // Helper function to normalize path separators to dots
        fn normalize_path(path: &str) -> String {
            path.replace('/', ".")
        }

        // Helper function to resolve relative path to absolute module path
        fn resolve_relative_path(import_path: &str, importing_mod: &str) -> String {
            if import_path.starts_with("./") {
                // Same directory import
                let relative = import_path.trim_start_matches("./");
                let normalized = normalize_path(relative);

                if importing_mod.is_empty() {
                    normalized
                } else {
                    format!("{importing_mod}.{normalized}")
                }
            } else if import_path.starts_with("../") {
                // Parent directory import
                // Start with the importing module parts as owned strings
                let mut module_parts: Vec<String> =
                    importing_mod.split('.').map(|s| s.to_string()).collect();

                let mut path_remaining: &str = import_path;

                // Navigate up for each '../'
                while path_remaining.starts_with("../") {
                    if !module_parts.is_empty() {
                        module_parts.pop();
                    }
                    // If we've gone above the module root, we just continue
                    // This handles cases like ../../../some/path from a shallow module
                    path_remaining = &path_remaining[3..];
                }

                // Add the remaining path
                if !path_remaining.is_empty() {
                    let normalized = normalize_path(path_remaining);
                    module_parts.extend(
                        normalized
                            .split('.')
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string()),
                    );
                }

                module_parts.join(".")
            } else {
                // Not a relative path, return as-is
                import_path.to_string()
            }
        }

        // Helper function to check if path matches with optional index resolution
        fn matches_with_index(candidate: &str, target: &str) -> bool {
            candidate == target || format!("{candidate}.index") == target
        }

        // Case 1: Exact match (most common case, check first for performance)
        if import_path == symbol_module_path {
            return true;
        }

        // Case 2: Only do complex matching if we have the importing module context
        if let Some(importing_mod) = importing_module {
            // TypeScript import resolution differs from Rust:
            // - Relative imports start with './' or '../'
            // - Absolute imports are package names or path aliases

            if import_path.starts_with("./") || import_path.starts_with("../") {
                // Resolve relative path to absolute module path
                let resolved = resolve_relative_path(import_path, importing_mod);

                // Check if it matches (with or without index)
                if matches_with_index(&resolved, symbol_module_path) {
                    return true;
                }
            }
            // else: bare module imports and scoped packages
            // These need exact match for now (TODO: implement proper resolution)
        }

        false
    }
}

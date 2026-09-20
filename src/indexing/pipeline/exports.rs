//! Resolve explicit module export slots without inventing declaration symbols.

use super::types::SymbolLookupCache;
use crate::parsing::{ExportResolution, FileExports};
use crate::{FileId, SymbolId};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A depth limit alone cannot bound a wide DAG of repeated star re-exports.
/// Exhaustion is propagated through the entire query so no partial branch can
/// become an apparently unique winner.
struct ExportTraversal {
    visiting: HashSet<(FileId, String)>,
    remaining: usize,
    exhausted: bool,
}

impl ExportTraversal {
    fn new() -> Self {
        Self {
            visiting: HashSet::new(),
            remaining: 4096,
            exhausted: false,
        }
    }
}

impl SymbolLookupCache {
    pub fn register_file_exports(&self, exports: FileExports) {
        if let Some((_, old)) = self.file_exports.remove(&exports.file_id) {
            self.export_file_paths.remove(Path::new(&old.file_path));
            if let Some(module) = old.module_path {
                if let Some(mut ids) = self.export_modules.get_mut(&module) {
                    ids.retain(|id| *id != old.file_id);
                }
            }
        }
        self.export_file_paths
            .insert(PathBuf::from(&exports.file_path), exports.file_id);
        if let Some(module) = &exports.module_path {
            self.export_modules
                .entry(module.clone())
                .or_default()
                .push(exports.file_id);
        }
        self.file_exports.insert(exports.file_id, exports);
    }

    pub(crate) fn replace_file_exports(&self, exports: Vec<FileExports>) {
        self.file_exports.clear();
        self.export_file_paths.clear();
        self.export_modules.clear();
        for surface in exports {
            self.register_file_exports(surface);
        }
    }

    pub fn resolve_export(
        &self,
        importing_file: FileId,
        source: &str,
        name: &str,
        extensions: &[&str],
    ) -> ExportResolution {
        let Some(importer) = self
            .file_exports
            .get(&importing_file)
            .map(|e| e.value().clone())
        else {
            return ExportResolution::Unknown;
        };
        let Some(path) = crate::parsing::paths::resolve_relative_specifier(
            Path::new(&importer.file_path),
            source,
        ) else {
            return ExportResolution::Unknown;
        };
        self.export_from_path(&path, name, extensions, &mut ExportTraversal::new())
    }

    pub fn resolve_module_export(
        &self,
        module: &str,
        name: &str,
        extensions: &[&str],
    ) -> ExportResolution {
        let exact = self
            .export_modules
            .get(module)
            .map(|e| e.value().clone())
            .unwrap_or_default();
        let files = if exact.is_empty() {
            self.export_modules
                .iter()
                .filter(|e| super::types::segment_suffix_match(e.key(), module))
                .flat_map(|e| e.value().clone())
                .collect()
        } else {
            exact
        };
        match files.as_slice() {
            [] => ExportResolution::Unknown,
            [file] => self.export_slot(*file, name, extensions, &mut ExportTraversal::new()),
            _ => ExportResolution::Ambiguous,
        }
    }

    fn export_from_path(
        &self,
        path: &Path,
        name: &str,
        extensions: &[&str],
        traversal: &mut ExportTraversal,
    ) -> ExportResolution {
        let mut accepted = Vec::<PathBuf>::new();
        // TypeScript source often uses the extension of the emitted JS file.
        // Perform extension substitution before accepting the output path.
        if extensions.contains(&"ts") || extensions.contains(&"mts") || extensions.contains(&"cts")
        {
            let substitutions: &[&str] = match path.extension().and_then(|e| e.to_str()) {
                Some("js") => &["ts", "tsx", "d.ts"],
                Some("jsx") => &["tsx", "ts", "d.ts"],
                Some("mjs") => &["mts", "d.mts"],
                Some("cjs") => &["cts", "d.cts"],
                _ => &[],
            };
            accepted.extend(substitutions.iter().map(|ext| path.with_extension(ext)));
        }
        accepted.push(path.to_path_buf());
        if path.extension().is_none() {
            if let Some(stem) = path.file_name().and_then(|n| n.to_str()) {
                for ext in extensions {
                    accepted.push(path.with_file_name(format!("{stem}.{ext}")));
                }
            }
        } else if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts")
        ) {
            // Dotted source stems (e.g. "./service.client") are not extensions.
            if let Some(stem) = path.file_name().and_then(|n| n.to_str()) {
                for ext in extensions {
                    accepted.push(path.with_file_name(format!("{stem}.{ext}")));
                }
            }
        }
        for ext in extensions {
            accepted.push(path.join(format!("index.{ext}")));
        }
        // File resolution has precedence: a.ts wins over a/index.ts.
        for candidate in accepted {
            if let Some(file) = self.export_file_paths.get(&candidate).map(|id| *id) {
                return self.export_slot(file, name, extensions, traversal);
            }
        }
        ExportResolution::Unknown
    }

    fn export_slot(
        &self,
        file: FileId,
        name: &str,
        extensions: &[&str],
        traversal: &mut ExportTraversal,
    ) -> ExportResolution {
        if traversal.remaining == 0 {
            traversal.exhausted = true;
            return ExportResolution::Ambiguous;
        }
        traversal.remaining -= 1;
        let key = (file, name.to_string());
        if traversal.visiting.len() >= 64 {
            traversal.exhausted = true;
            return ExportResolution::Ambiguous;
        }
        if !traversal.visiting.insert(key.clone()) {
            return ExportResolution::Missing;
        }
        let result = self.export_slot_inner(file, name, extensions, traversal);
        traversal.visiting.remove(&key);
        if traversal.exhausted {
            ExportResolution::Ambiguous
        } else {
            result
        }
    }

    fn export_slot_inner(
        &self,
        file: FileId,
        name: &str,
        extensions: &[&str],
        traversal: &mut ExportTraversal,
    ) -> ExportResolution {
        let Some(surface) = self.file_exports.get(&file).map(|e| e.value().clone()) else {
            return ExportResolution::Unknown;
        };
        let (head, tail) = name
            .split_once('.')
            .map_or((name, None), |(head, tail)| (head, Some(tail)));
        let explicit: Vec<_> = surface
            .exports
            .iter()
            .filter(|e| !e.is_glob && e.exported_name == head)
            .collect();
        let mut targets = HashMap::<SymbolId, bool>::new();
        let mut ambiguous = false;
        if !explicit.is_empty() {
            for export in explicit {
                if traversal.exhausted {
                    break;
                }
                if let Some(source) = export.source.as_deref() {
                    let source_name = if export.is_namespace {
                        let Some(member) = tail else {
                            continue;
                        };
                        member.to_string()
                    } else {
                        let local = export.local_name.as_deref().unwrap_or(head);
                        tail.map_or_else(|| local.to_string(), |member| format!("{local}.{member}"))
                    };
                    if let Some(path) = crate::parsing::paths::resolve_relative_specifier(
                        Path::new(&surface.file_path),
                        source,
                    ) {
                        Self::merge_export_target(
                            &mut targets,
                            &mut ambiguous,
                            self.export_from_path(&path, &source_name, extensions, traversal),
                            export.is_type_only,
                        );
                    }
                } else if let Some(local_name) =
                    export.local_name.as_deref().filter(|_| tail.is_none())
                {
                    for id in self.symbols_in_file(file) {
                        if self.get(id).is_some_and(|s| {
                            s.name.as_ref() == local_name
                                && matches!(
                                    s.scope_context,
                                    None | Some(crate::symbol::ScopeContext::Module)
                                        | Some(crate::symbol::ScopeContext::Global)
                                )
                        }) {
                            targets
                                .entry(id)
                                .and_modify(|only| *only &= export.is_type_only)
                                .or_insert(export.is_type_only);
                        }
                    }
                }
            }
        } else if head != "default" {
            for export in surface.exports.iter().filter(|e| e.is_glob) {
                if traversal.exhausted {
                    break;
                }
                if let Some(path) = export.source.as_deref().and_then(|source| {
                    crate::parsing::paths::resolve_relative_specifier(
                        Path::new(&surface.file_path),
                        source,
                    )
                }) {
                    Self::merge_export_target(
                        &mut targets,
                        &mut ambiguous,
                        self.export_from_path(&path, name, extensions, traversal),
                        export.is_type_only,
                    );
                }
            }
        }
        if ambiguous || targets.len() > 1 {
            ExportResolution::Ambiguous
        } else if let Some((id, type_only)) = targets.into_iter().next() {
            if type_only {
                ExportResolution::TypeOnly(id)
            } else {
                ExportResolution::Found(id)
            }
        } else {
            ExportResolution::Missing
        }
    }

    fn merge_export_target(
        targets: &mut HashMap<SymbolId, bool>,
        ambiguous: &mut bool,
        resolution: ExportResolution,
        force_type_only: bool,
    ) {
        let (id, type_only) = match resolution {
            ExportResolution::Found(id) => (id, force_type_only),
            ExportResolution::TypeOnly(id) => (id, true),
            ExportResolution::Ambiguous => {
                *ambiguous = true;
                return;
            }
            _ => return,
        };
        targets
            .entry(id)
            .and_modify(|only| *only &= type_only)
            .or_insert(type_only);
    }
}

//! Explicit ECMAScript export slots, independent of local symbol names.

use crate::{FileId, SymbolId};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    /// Public name, `default`, or `*` for a star re-export.
    pub exported_name: String,
    /// Local declaration name, or the imported name in a re-export.
    pub local_name: Option<String>,
    /// Module specifier for a re-export. Local exports have no source.
    pub source: Option<String>,
    pub is_glob: bool,
    pub is_type_only: bool,
    #[serde(default)]
    pub is_namespace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileExports {
    pub file_id: FileId,
    pub file_path: String,
    #[serde(default)]
    pub module_path: Option<String>,
    pub exports: Vec<Export>,
}

/// `Unknown` means this cache has no export evidence for the file. Missing
/// and ambiguous slots must not fall through to a same-named declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportResolution {
    Unknown,
    Missing,
    Found(SymbolId),
    /// The declaration is available for type relationships, but every
    /// export path to this slot erases its runtime value.
    TypeOnly(SymbolId),
    Ambiguous,
}

pub(crate) fn link_imported_exports(exports: &mut [Export], imports: &[crate::parsing::Import]) {
    for export in exports.iter_mut().filter(|e| e.source.is_none()) {
        let Some(local) = export.local_name.as_deref() else {
            continue;
        };
        if let Some(import) = imports
            .iter()
            .find(|import| import.alias.as_deref() == Some(local))
        {
            export.source = Some(import.path.clone());
            export.local_name = if import.is_glob {
                None
            } else {
                Some(import.imported_name.as_deref().unwrap_or(local).to_string())
            };
            export.is_namespace = import.is_glob;
            export.is_type_only |= import.is_type_only;
        }
    }
}

pub(crate) fn ecmascript_exports(root: Node, code: &str) -> Vec<Export> {
    let mut exports = Vec::new();
    for statement in root.named_children(&mut root.walk()) {
        if statement.kind() != "export_statement" {
            continue;
        }
        let source = statement.child_by_field_name("source").map(|node| {
            code[node.byte_range()]
                .trim_matches(['\'', '"'])
                .to_string()
        });
        let is_type_only = statement
            .children(&mut statement.walk())
            .any(|n| n.kind() == "type");
        let is_default = statement
            .children(&mut statement.walk())
            .any(|n| n.kind() == "default");
        if is_default {
            let local_name = statement
                .named_children(&mut statement.walk())
                .find_map(|child| {
                    if child.kind() == "identifier" {
                        Some(code[child.byte_range()].to_string())
                    } else {
                        child
                            .child_by_field_name("name")
                            .map(|name| code[name.byte_range()].to_string())
                    }
                });
            exports.push(Export {
                exported_name: "default".into(),
                local_name,
                source,
                is_glob: false,
                is_type_only,
                is_namespace: false,
            });
            continue;
        }
        let mut handled = false;
        for child in statement.named_children(&mut statement.walk()) {
            if child.kind() == "export_clause" {
                handled = true;
                for spec in child.named_children(&mut child.walk()) {
                    if spec.kind() != "export_specifier" {
                        continue;
                    }
                    let Some(name) = spec.child_by_field_name("name") else {
                        continue;
                    };
                    let alias = spec.child_by_field_name("alias").unwrap_or(name);
                    exports.push(Export {
                        exported_name: code[alias.byte_range()]
                            .trim_matches(['\'', '"'])
                            .to_string(),
                        local_name: Some(
                            code[name.byte_range()]
                                .trim_matches(['\'', '"'])
                                .to_string(),
                        ),
                        source: source.clone(),
                        is_glob: false,
                        is_type_only: is_type_only
                            || spec.children(&mut spec.walk()).any(|n| n.kind() == "type"),
                        is_namespace: false,
                    });
                }
            } else if child.kind() == "namespace_export" {
                handled = true;
                if let Some(name) = child.named_child(0) {
                    exports.push(Export {
                        exported_name: code[name.byte_range()].to_string(),
                        local_name: None,
                        source: source.clone(),
                        is_glob: false,
                        is_type_only,
                        is_namespace: true,
                    });
                }
            }
        }
        if handled {
            continue;
        }
        if statement
            .children(&mut statement.walk())
            .any(|n| n.kind() == "*")
        {
            exports.push(Export {
                exported_name: "*".into(),
                local_name: None,
                source,
                is_glob: true,
                is_type_only,
                is_namespace: false,
            });
            continue;
        }
        // Exported declarations bind their declared names. Destructuring and
        // anonymous expressions remain unbound until their symbols are modeled.
        for declaration in statement.named_children(&mut statement.walk()) {
            if let Some(name) = declaration.child_by_field_name("name") {
                let name = code[name.byte_range()].to_string();
                exports.push(Export {
                    exported_name: name.clone(),
                    local_name: Some(name),
                    source: None,
                    is_glob: false,
                    is_type_only: is_type_only
                        || matches!(
                            declaration.kind(),
                            "interface_declaration" | "type_alias_declaration"
                        ),
                    is_namespace: false,
                });
            } else if matches!(
                declaration.kind(),
                "lexical_declaration" | "variable_declaration"
            ) {
                for variable in declaration.named_children(&mut declaration.walk()) {
                    if let Some(name) = variable
                        .child_by_field_name("name")
                        .filter(|n| n.kind() == "identifier")
                    {
                        let name = code[name.byte_range()].to_string();
                        exports.push(Export {
                            exported_name: name.clone(),
                            local_name: Some(name),
                            source: None,
                            is_glob: false,
                            is_type_only,
                            is_namespace: false,
                        });
                    }
                }
            }
        }
    }
    exports
}

from pathlib import Path

parser = Path("src/parsing/typescript/parser.rs")
s = parser.read_text()
old = '''                            if ni.kind() == "import_specifier" {
                                let mut sp = ni.walk();
                                let mut local: Option<String> = None;
                                // Prefer the aliased local name if present
                                for part in ni.children(&mut sp) {
                                    if part.kind() == "identifier" {
                                        local = Some(code[part.byte_range()].to_string());
                                    }
                                }
                                imports.push(Import {
                                    path: source_path.to_string(),
                                    alias: local,
                                    file_id,
                                    is_glob: false,
                                    is_type_only,
                                });
                            }
'''
new = '''                            if ni.kind() == "import_specifier" {
                                let mut sp = ni.walk();
                                let identifiers: Vec<String> = ni
                                    .children(&mut sp)
                                    .filter(|part| part.kind() == "identifier")
                                    .map(|part| code[part.byte_range()].to_string())
                                    .collect();
                                let imported = identifiers.first().cloned();
                                let local = identifiers.last().cloned();

                                // Persist the full import identity. This matches
                                // Codanna's existing full-path convention used by
                                // languages such as Rust (module::Symbol).
                                let path = imported.as_ref().map_or_else(
                                    || source_path.to_string(),
                                    |name| format!("{source_path}::{name}"),
                                );
                                imports.push(Import {
                                    path,
                                    alias: local,
                                    file_id,
                                    is_glob: false,
                                    is_type_only,
                                });
                            }
'''
if old not in s:
    raise SystemExit("TypeScript named import block not found")
parser.write_text(s.replace(old, new, 1))

behavior = Path("src/parsing/typescript/behavior.rs")
s = behavior.read_text()
old = '''        for import in imports {
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

            // Path-domain arm first: relative specifiers resolve by file
'''
new = '''        for import in imports {
            // Named TypeScript imports persist as `module::ImportedName`,
            // while `alias` is the name exposed inside the importing file.
            let (module_import_path, imported_name) = import
                .path
                .rsplit_once("::")
                .map_or((import.path.as_str(), None), |(module, name)| {
                    (module, Some(name))
                });

            let local_name = import.alias.clone().unwrap_or_else(|| {
                imported_name.map_or_else(
                    || {
                        module_import_path
                            .split('/')
                            .next_back()
                            .or_else(|| module_import_path.split('.').next_back())
                            .unwrap_or(module_import_path)
                            .to_string()
                    },
                    str::to_string,
                )
            });
            let lookup_name = imported_name.unwrap_or(&local_name);

            // Path-domain arm first: relative specifiers resolve by file
'''
if old not in s:
    raise SystemExit("TypeScript behavior loop header not found")
s = s.replace(old, new, 1)
replacements = [
    (
        "self.resolve_relative_import(cache, &local_name, &import.path, f, extensions)",
        "self.resolve_relative_import(cache, lookup_name, module_import_path, f, extensions)",
        1,
    ),
    (
        "enhancer.enhance_import_path(&import.path, file_id)",
        "enhancer.enhance_import_path(module_import_path, file_id)",
        1,
    ),
    (
        "normalize_import(&import.path, &importing_module.clone().unwrap_or_default())",
        "normalize_import(module_import_path, &importing_module.clone().unwrap_or_default())",
        2,
    ),
    (
        "for id in cache.lookup_candidates(&local_name) {",
        "for id in cache.lookup_candidates(lookup_name) {",
        1,
    ),
]
for before, after, count in replacements:
    if s.count(before) < count:
        raise SystemExit(f"expected behavior expression not found: {before}")
    s = s.replace(before, after, count)
behavior.write_text(s)

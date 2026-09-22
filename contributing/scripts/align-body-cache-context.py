#!/usr/bin/env python3
"""Reviewed local correction for the planner/CLI configuration-path mismatch."""
import pathlib

path = pathlib.Path("src/config/mod.rs")
source = path.read_text()
old = "                .then(|| config_dir.parent().map(PathBuf::from))\n"
new = """                .then(|| {
                    config_dir.parent().map(|root| {
                        // A bare .codanna/settings.toml has an empty lexical
                        // parent. Treat it as the current directory so the
                        // usual canonicalization resolves the same workspace
                        // as an absolute --config path and the offline planner.
                        if root.as_os_str().is_empty() {
                            PathBuf::from(".")
                        } else {
                            root.to_path_buf()
                        }
                    })
                })
"""
if source.count(old) != 1:
    raise SystemExit("Config root inference no longer matches the reviewed source")
path.write_text(source.replace(old, new, 1))

path = pathlib.Path("src/semantic/simple.rs")
source = path.read_text()
old = "    pub(crate) fn store_embeddings_with_inputs(\n"
if source.count(old) != 1:
    raise SystemExit("Expected the legacy test-only input storage helper")
# The integrated runtime uses store_shared_embedding or store_symbol_segments.
# Retain the old helper for the existing fixed-vector tests, not release builds.
path.write_text(source.replace(old, "    #[cfg(test)]\n" + old, 1))
print("Resolved bare relative config roots; retained legacy storage helper for unit tests")

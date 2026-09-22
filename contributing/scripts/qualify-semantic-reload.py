#!/usr/bin/env python3
"""One-time pinned repair recipe; no remote refs, providers, or index operations."""
import hashlib
import json
import pathlib
import subprocess
import sys

path = pathlib.Path("src/indexing/facade.rs")
expected = "465e1d448df03fdd0c9f063110b7a0e6bbf11d9a"
actual = subprocess.check_output(["git", "hash-object", str(path)], text=True).strip()
if actual != expected:
    raise SystemExit(f"Refusing changed facade: {actual} != {expected}")
source = path.read_text()
old = '''                Err(e) => {
                    // Other errors (missing file, corrupt data) — warn and continue
                    // without semantic search rather than blocking startup.
                    tracing::warn!("Failed to load semantic search, continuing without it: {e}");
                }
            }
        }
        Ok(false)
    }
'''
new = '''                Err(e) => {
                    // Retain old data for recovery, but never serve or republish
                    // it after a failed reload. Lexical startup remains usable.
                    self.semantic_incompatible = true;
                    tracing::warn!("Failed to load semantic search, continuing without it: {e}");
                }
            }
        } else if self.semantic_search.is_some() {
            // A manifest removed after a previous load invalidates that loaded
            // generation. A genuinely absent first-load store stays optional.
            self.semantic_incompatible = true;
        }
        Ok(false)
    }
'''
if source.count(old) != 1:
    raise SystemExit("Reload target is not unique")
path.write_text(source.replace(old, new))
print(json.dumps({"file": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}))

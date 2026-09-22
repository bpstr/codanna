#!/usr/bin/env python3
"""One-time exact-ref integration. Never pushes or updates a remote branch."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess

PINS = {
    "related": "c22eb9836367e5aadde83179f5528c89c5dc2249",
    "scope": "43c15e159c61bda5f6c44be1a861f96652f606d2",
    "lexical": "c6f0a6a93372c57460fef870fb780cbc9ad9d044",
}


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], text=True, check=check, capture_output=True)


def main() -> None:
    if git("status", "--porcelain").stdout.strip():
        raise SystemExit("Refusing to integrate into a dirty checkout")
    git("merge-base", "--is-ancestor", PINS["related"], "HEAD")
    original = git("rev-parse", "HEAD").stdout.strip()
    git("config", "user.name", "Integration verification")
    git("config", "user.email", "integration-verification@users.noreply.github.com")
    for label in ("scope", "lexical"):
        sha = PINS[label]
        git("fetch", "--no-tags", "origin", sha)
        result = git("merge", "--no-commit", "--no-ff", sha, check=False)
        print(result.stdout, result.stderr, flush=True)
        if result.returncode:
            conflicts = git("diff", "--name-only", "--diff-filter=U").stdout.splitlines()
            # The only admissible manual union is independent module declarations.
            if conflicts != ["src/storage/tantivy/mod.rs"] or label != "lexical":
                raise SystemExit(f"Unreviewed merge conflicts: {conflicts}")
            path = Path(conflicts[0])
            ours = git("show", f":2:{path}").stdout
            theirs = git("show", f":3:{path}").stdout
            for declaration in ("mod linguistic_coverage;", "mod ranking_efficiency;"):
                if declaration not in theirs:
                    raise SystemExit(f"Expected dependency declaration missing: {declaration}")
                if declaration not in ours:
                    ours = ours.replace("mod query;", f"mod query;\n{declaration}")
            path.write_text(ours)
            git("add", str(path))
        if git("diff", "--name-only", "--diff-filter=U").stdout.strip():
            raise SystemExit("Unresolved integration conflicts")
        git("diff", "--cached", "--check")
        git("commit", "--no-verify", "-m", f"Verify pinned {label} integration locally")
    print("integration_pins=" + json.dumps({"staged_head": original, **PINS}), flush=True)


if __name__ == "__main__":
    main()

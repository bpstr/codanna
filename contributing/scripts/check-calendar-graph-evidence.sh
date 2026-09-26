#!/usr/bin/env bash
# Public-boundary fixtures only: semantic indexing is disabled in the test setup.
set -euo pipefail
cd "$(dirname "$0")/../.."

printf 'source_revision='
git rev-parse HEAD
rustc --version --verbose
sha256sum \
  src/mcp/tools/symbols.rs \
  tests/calendar_graph_evidence.rs \
  tests/fixtures/calendar_graph_evidence/workspace/active/calendar.ts \
  tests/fixtures/calendar_graph_evidence/workspace/reference/calendar.ts \
  contributing/retrieval/calendar-graph-evidence/cases.json

python3 - <<'PY'
import hashlib
import json
import pathlib

oracle = json.loads(pathlib.Path(
    "contributing/retrieval/calendar-graph-evidence/cases.json"
).read_text())
for name, expected in oracle["files"].items():
    actual = hashlib.sha256(pathlib.Path(name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"Fixture digest mismatch: {name}: {actual} != {expected}")
print("fixture_digests=verified", flush=True)
PY

build_events=$(mktemp)
trap 'rm -f "$build_events"' EXIT
# On compilation failure, stop before hashing or running any cached executable.
cargo test --locked --test calendar_graph_evidence --no-run \
  --message-format=json-render-diagnostics > "$build_events"
python3 - "$build_events" <<'PY'
import hashlib
import json
import pathlib
import subprocess
import sys

executables = set()
for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
    event = json.loads(line)
    if (event.get("reason") == "compiler-artifact"
            and event.get("target", {}).get("name") == "calendar_graph_evidence"
            and event.get("executable")):
        executables.add(event["executable"])
if len(executables) != 1:
    raise SystemExit("Expected exactly one current graph-contract executable")
executable = pathlib.Path(executables.pop()).resolve(strict=True)
digest = hashlib.sha256()
with executable.open("rb") as binary:
    for block in iter(lambda: binary.read(1024 * 1024), b""):
        digest.update(block)
print(f"test_executable_sha256={digest.hexdigest()}  {executable}", flush=True)
# Do not invoke Cargo again: build scripts may rebuild and invalidate the digest.
status = subprocess.run([str(executable), "--nocapture"], check=False).returncode
print(f"test_exit_status={status}", flush=True)
raise SystemExit(status if status >= 0 else 128 - status)
PY

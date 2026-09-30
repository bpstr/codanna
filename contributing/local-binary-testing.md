# Local binary testing manual

Use this checklist for a locally built Codanna candidate before creating its
version tag or replacing an installed executable. Keep the candidate separate
from the installed binary throughout testing. The [release testing manual](release-testing.md)
adds release-wide CI, retrieval/performance comparisons and Assign activation gates.
Passing the smoke test below alone is not release qualification.

## 1. Freeze the source and save rollback executables

Start from a clean commit. Record `git rev-parse HEAD`, `git status --porcelain`,
`rustc --version`, `cargo --version`, architecture, build features and free disk
space. Use a new evidence directory outside every indexed source root. Save the
installed executables there, recording resolved paths, permissions, versions and
SHA-256 hashes. Save matching auxiliary tools too; do not infer their version
from the main executable. Record running MCP processes and their workspaces.

Estimate build/test disk use from the selected targets and existing caches.
There is no fixed free-space minimum. Stop if space cannot accommodate the
remaining work; preserve indexes, model caches and qualification evidence.

## 2. Run the code gates and build a clean candidate

Read Assign's paid-inference testing policy first. Never source `.secrets` or
pass provider credentials, subscription proxies or fixture-recording flags to
tests. These commands assume the normal Rust toolchain is on `PATH`:

```sh
env -i HOME="$HOME" PATH="$PATH" TMPDIR=/tmp \
  ./contributing/scripts/quick-check.sh
env -i HOME="$HOME" PATH="$PATH" TMPDIR=/tmp \
  ./contributing/scripts/full-test.sh
env -i HOME="$HOME" PATH="$PATH" TMPDIR=/tmp \
  cargo test --locked --all-features --no-fail-fast
env -i HOME="$HOME" PATH="$PATH" TMPDIR=/tmp \
  cargo build --locked --release --all-features --bins
```

Capture complete logs and exit codes. Record ignored tests separately. Exact-source
results already obtained in the same qualification may be reused with their
source/feature identities; a skipped CI release job is not a tested build.
All-feature builds include accelerator support, but compilation does not prove
native device execution. Automated inference contracts use prepared fixtures and
mocked transports. Local-model development measurements require the separate
explicit, offline procedure in the release manual.

Copy the release executables to the evidence directory. Record their hashes and
run the candidate's `--version`; reject an unexpected version or `-dirty` stamp.
Use this copied artifact for subsequent tests so another build cannot change it.

## 3. Smoke test the exact executable without a model

Set `candidate` to the absolute path of the copied executable. The following
creates a fresh workspace and HOME, disables semantic inference and exercises
fresh indexing, graph lookup, a second-process reopen and actual stdio MCP startup.
Run it under `bash`; a failed command stops the test. Preserve the output directory.

```bash
set -euo pipefail
candidate=/absolute/path/to/evidence/codanna
scratch=$(mktemp -d "${TMPDIR:-/tmp}/codanna-binary-smoke.XXXXXX")
scratch=$(cd "$scratch" && pwd -P)
mkdir -p "$scratch/home" "$scratch/workspace/src" "$scratch/workspace/.codanna"
cat > "$scratch/workspace/src/probe.rs" <<'RUST'
pub fn smoke_leaf() -> u32 { 42 }
pub fn smoke_entry() -> u32 { smoke_leaf() }
RUST
cat > "$scratch/workspace/.codanna/settings.toml" <<'TOML'
index_path = ".codanna/index"
[semantic_search]
enabled = false
TOML
run_candidate() {
  env -i HOME="$scratch/home" PATH="$PATH" TMPDIR=/tmp "$candidate" "$@"
}
cd "$scratch/workspace"
run_candidate --version > "$scratch/version.txt"
run_candidate --help > "$scratch/help.txt"
run_candidate index src --no-progress > "$scratch/index.txt" 2>&1
run_candidate retrieve symbol smoke_entry > "$scratch/symbol.txt"
run_candidate retrieve calls smoke_entry > "$scratch/calls.txt"
run_candidate retrieve symbol smoke_entry > "$scratch/reopened-symbol.txt"
run_candidate index src --no-progress > "$scratch/unchanged-index.txt" 2>&1
run_candidate mcp get_index_info --json > "$scratch/index-info.json"
run_candidate mcp-test > "$scratch/mcp.txt" 2>&1
printf 'Evidence: %s\n' "$scratch"
```

Check the actual results: both lookups identify `smoke_entry` in `src/probe.rs`,
the calls response contains `smoke_leaf`, and metadata reports one source file,
two symbols and one call relationship. A zero exit code alone is insufficient.
The second lookup must work without rebuilding. The unchanged indexing run must
preserve counts. This test does not exercise embeddings or document vectors.

## 4. Run regression and recovery witnesses against the artifact

From the repository root, with absolute candidate/evidence paths:

```sh
python3.11 contributing/retrieval/run.py --profile structural \
  --codanna /absolute/path/to/evidence/codanna \
  --knowledge /absolute/path/to/evidence/codanna-knowledge \
  --out /absolute/path/to/new-structural-evidence
python3.11 contributing/retrieval/qualify-code-recovery.py \
  --codanna /absolute/path/to/evidence/codanna \
  --out /absolute/path/to/new-recovery-evidence
```

The recovery witness requires macOS and deliberately kills only its own scratch
indexer. Its report must show resumed/forced count parity and every sampled call
edge. Never point it at an active index. Keep the first failure if a harness or
implementation needs repair. Run the same relevant fixture checks against the
saved baseline to distinguish new regressions from known failures.

For behavior affected by the release, retain the corresponding automated
regressions: body-only edits and cache reuse, config-only alias rebinding,
create/rename/delete, missing/corrupt persistence, document churn, and watcher
recovery. Prepared embedding results prove contracts, not model quality or
real indexing speed. Use the release manual's unchanged manifests and alternating
trials for any measured retrieval or throughput claim.

## 5. Decide whether to tag and replace

Before promotion, record pass/fail/not-run for each applicable gate, exact source
and binary hashes, CI status, compatibility requirements, known limitations and
rollback paths. A new correctness regression, unexplained performance regression,
or failed required gate blocks promotion. Do not label an untested platform as
qualified. Preserve body-v2 and existing model identities unless a separately
approved migration explicitly changes them.

Only after qualification, create the version tag and replace the installed file
using a sibling temporary executable and atomic rename. Verify the installed
SHA-256 equals the tested candidate, then repeat version/help and the scratch
smoke using the installed path. Keep the backup. Replace auxiliary tools only
with artifacts from the same qualified build.

Replacing a file does not upgrade running servers. Identify and restart only
intended consumers after compatibility is established; report any still using
the old binary. Index activation/rebuilding is a separate decision governed by
the release manual. Do not mutate the live Assign index as a binary smoke test.

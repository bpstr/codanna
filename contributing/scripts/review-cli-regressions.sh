#!/usr/bin/env bash
# Offline replacements for the legacy scripts; propagate Cargo and assertion failures.
set -euo pipefail
cd "$(dirname "$0")/../.."
log="$(mktemp)"
trap 'rm -f "$log"' EXIT
CARGO_TERM_COLOR=never cargo test --locked --all-features --test cli_tests test_review_cli_contracts -- --nocapture 2>&1 | tee "$log"
grep -Eq 'test result: ok\. [1-9][0-9]* passed;' "$log" || {
  echo 'CLI regression filter executed no tests' >&2
  exit 1
}

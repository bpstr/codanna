#!/usr/bin/env bash
# Deliberately restricted to deterministic fixtures, mock embeddings, and local transports.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
export CARGO_TERM_COLOR=never

run_tests() {
  local log
  log="$(mktemp)"
  if ! cargo test --locked --all-features "$@" -- --nocapture --test-threads=2 2>&1 | tee "$log"; then
    rm -f "$log"
    return 1
  fi
  # A renamed filter must not turn this gate into a zero-test success.
  if ! grep -Eq 'test result: ok\. [1-9][0-9]* passed;' "$log"; then
    echo "No tests passed for filter: $*" >&2
    rm -f "$log"
    return 1
  fi
  rm -f "$log"
}

cargo test --locked --all-features --no-run
run_tests --lib hardening_review_
for module in \
  'documents::' \
  'parsing::factory::' \
  'symbol::' \
  'vector::storage::' \
  'plugins::resolver::' \
  'mcp::requests::' \
  'indexing::pipeline::stages::write::'
do
  run_tests --lib "$module"
done
run_tests --test cli_tests test_serve_http_sessionless
node --test tests/security/plugin-boundaries.test.cjs

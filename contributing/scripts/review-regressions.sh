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
status=0
run_tests --lib hardening_review_ || status=1
run_tests --lib hardening_final_ || status=1
run_tests --test hardening_symbol_cache hardening_symbol_cache_ || status=1
for module in \
  'documents::' \
  'parsing::factory::' \
  'symbol::' \
  'vector::storage::' \
  'plugins::resolver::' \
  'mcp::requests::' \
  'indexing::pipeline::stages::write::' \
  'indexing::pipeline::stages::context::' \
  'indexing::walker::'
do
  run_tests --lib "$module" || status=1
done
run_tests --test cli_tests test_serve_http_sessionless || status=1
run_tests --test cli_tests test_review_cli_contracts || status=1
run_tests --test parsers_tests test_typescript_alias_resolution || status=1
run_tests --test parsers_tests test_typescript_pipeline_resolution || status=1
run_tests --test integration_tests test_parse_command || status=1
run_tests --test exploration_tests abi15_grammar_audit || status=1
node --test tests/security/plugin-boundaries.test.cjs || status=1

exit "$status"

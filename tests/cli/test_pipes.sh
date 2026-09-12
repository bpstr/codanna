#!/usr/bin/env bash
# Compatibility entrypoint: the assertions now live in test_review_cli_contracts.rs.
# Uses an isolated mock-free, semantic-disabled index; does not inspect your repository.
set -euo pipefail
cd "$(dirname "$0")/../.."
exec bash contributing/scripts/review-cli-regressions.sh

#!/usr/bin/env python3
"""Require executed body-input and persistence checks, not zero-test success."""
import json
import pathlib
import re
import sys


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: check-body-input-validation.py TEST_LOG")
    text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", text)
    if len(results) != 6 or any(int(passed) == 0 or int(failed) or int(ignored)
                                for passed, failed, ignored in results):
        raise SystemExit(f"Expected six nonempty successful test selections: {results}")
    rows = [json.loads(line.split("repository_body_input=", 1)[1])
            for line in text.splitlines() if "repository_body_input=" in line]
    summaries = [json.loads(line.split("repository_body_summary=", 1)[1])
                 for line in text.splitlines() if "repository_body_summary=" in line]
    if len(rows) != 20 or len(summaries) != 1:
        raise SystemExit("Expected 20 question/input measurements and one summary")
    if len({(row["task"], row["variant"]) for row in rows}) != 20:
        raise SystemExit("Duplicate or missing question/input measurement")
    summary = summaries[0]
    if (summary["owners"], summary["legacy_eligible"], summary["body_captured"]) != (10, 6, 10):
        raise SystemExit(f"Owner capture changed: {summary}")
    if summary["provider_requests"] != 0 or summary["retrieval_quality"] is not None:
        raise SystemExit("This test must remain provider-free input coverage, not model evaluation")
    if any(not row["source_excerpts_verified"] or row["vector_generated"] for row in rows):
        raise SystemExit("Source provenance or no-generation contract failed")
    print(f"Verified {sum(int(row[0]) for row in results)} tests; all ten owner excerpts captured.")


if __name__ == "__main__":
    main()

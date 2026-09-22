#!/usr/bin/env python3
"""Guard measured recoveries; this is not the original overall quality threshold."""
import json
import pathlib
import sys


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: check-task-relevance-recovery.py TEST_LOG")
    lines = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
    cases = [json.loads(line.split("repository_task_relevance=", 1)[1])
             for line in lines if "repository_task_relevance=" in line]
    summaries = [json.loads(line.split("repository_task_summary=", 1)[1])
                 for line in lines if "repository_task_summary=" in line]
    if len(cases) != 20 or len(summaries) != 1:
        raise SystemExit("Expected exactly 20 measured questions and one summary")
    if summaries[0]["oracle_sha256"] != "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6":
        raise SystemExit("Original question/owner oracle changed")
    indexed = {(row["task"], row["variant"]): row for row in cases}
    if len(indexed) != 20:
        raise SystemExit("Duplicate or missing task/family outcomes")
    required = [
        ("growing-file", "operational"), ("rename-ambiguity", "operational"),
        ("recall-identity", "paraphrase"), ("cache-identity", "operational"),
        ("cache-atomic-save", "operational"), ("adaptive-batch", "operational"),
        ("partial-walk", "operational"), ("recall-identity", "operational"),
    ]
    for key in required:
        rank = indexed[key]["rank_at_5"]
        if not isinstance(rank, int) or not 1 <= rank <= 5:
            raise SystemExit(f"Lost a recorded recovery: {key}: {rank}")
    print("Eight targeted recoveries retained; the original 90% quality gate is separate.")


if __name__ == "__main__":
    main()

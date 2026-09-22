#!/usr/bin/env python3
"""Guard measured query compatibility, not the original relevance quality gate."""
from __future__ import annotations

import json
from pathlib import Path
import sys

ORACLE = "129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6"
DIRECT = {
    ("growing-file", 0), ("rename-ambiguity", 0), ("partial-walk", 0),
    ("recall-identity", 0), ("recall-identity", 1), ("cache-identity", 0),
    ("cache-atomic-save", 0), ("adaptive-batch", 0),
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def observations(log: str, marker: str) -> list[dict]:
    rows = []
    for line in log.splitlines():
        if marker not in line:
            continue
        payload = line.split(marker, 1)[1]
        value, end = json.JSONDecoder().raw_decode(payload)
        require(not payload[end:].strip() and isinstance(value, dict),
                f"Malformed observation for {marker}")
        rows.append(value)
    return rows


def check(log: str) -> dict:
    tasks = observations(log, "repository_task_summary=")
    related = observations(log, "ticket_related_summary=")
    require(len(tasks) == 1 and len(related) == 1,
            "Exactly one direct and one related summary must execute")
    task, extra = tasks[0], related[0]
    require(task["oracle_sha256"] == ORACLE, "Frozen oracle changed")
    require(task["queries"] == 20 and task["indexed_symbols"] == 152,
            "Frozen query/corpus count changed")
    require(task["operational_hit_at_5"] == 7 and task["paraphrase_hit_at_5"] == 1,
            "Direct facade compatibility changed")
    require(extra["queries"] == 20 and extra["direct_hit_at_5"] == [7, 1],
            "Direct ticket compatibility changed")
    require(extra["direct_or_related_hit_at_up_to_11"] == [9, 1],
            "Related evidence coverage changed")
    require(extra["direct_results_unchanged"] is True,
            "Related opt-in altered direct results")
    require(extra["quality_gate_passed"] is False and extra["paid_inference"] is False,
            "Measurement must not claim the quality gate or paid evaluation")
    require(len(observations(log, "repository_task_relevance=")) == 20,
            "Missing direct-query outcomes")
    cases = observations(log, "ticket_related_case=")
    require(len(cases) == 20 and len({(r["task"], r["family"]) for r in cases}) == 20,
            "Missing or duplicated ticket-query outcomes")
    direct = {(r["task"], r["family"]) for r in cases if r["direct_rank_at_5"] is not None}
    expanded = {(r["task"], r["family"]) for r in cases
                if r["direct_rank_at_5"] is not None or r["related_rank_at_6"] is not None}
    require(direct == DIRECT, "Known direct owners changed")
    require(expanded == DIRECT | {("recall-timeout", 0), ("foreign-recall", 0)},
            "Known related owners changed")
    retries = observations(log, "ticket_related_generation_retry=")
    require(extra["generation_retries"] == len(retries), "Unreported reader retries")
    for row in retries:
        require(row["related"]["status"] in {
            "not_run_generation_mismatch", "discarded_generation_changed"
        } and row["related"]["items"] == [], "Invalidated graph evidence was retained")
    return {
        "known_direct_hit_at_5": 8, "known_direct_or_related": 10, "queries": 20,
        "generation_retries": len(retries), "regression_contract_passed": True,
        "original_quality_gate_passed": False,
    }


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: check-scoped-ticket-relevance.py TEST_LOG")
    print(json.dumps(check(Path(sys.argv[1]).read_text()), sort_keys=True))

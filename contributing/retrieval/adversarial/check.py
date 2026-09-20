#!/usr/bin/env python3
"""Check fixture coverage and record a completed, model-free Cargo test run.

This script never starts Cargo, loads models, or invokes an embedding provider.
It refuses to count an incomplete log, skipped test, or build error as a pass.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
CASE_PATH = HERE / "cases.json"
BASELINE_PATH = HERE / "baseline.json"
RESULTS_PATH = HERE / "results.json"
CHECKLIST_PATH = HERE / "CHECKLIST.md"


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def repo_file(relative: str) -> Path:
    path = (REPO / relative).resolve()
    if not path.is_relative_to(REPO) or not path.is_file():
        raise ValueError(f"Missing or invalid repository file: {relative}")
    return path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def validate(manifest: dict, baseline: dict, results: dict) -> None:
    require(manifest["schema_version"] == 1, "Unsupported manifest schema")
    cases = manifest["cases"]
    ids = [case["id"] for case in cases]
    tests = [case["test"] for case in cases]
    require(len(set(ids)) == len(ids), "Duplicate case IDs")
    require(len(set(tests)) == len(tests), "Duplicate test names")
    gateway = repo_file(f"tests/{manifest['target']}.rs").read_text()
    modules = re.findall(r'#\[path\s*=\s*"([^"]+)"\]\s*mod\s+(\w+)\s*;', gateway)
    require(bool(modules), "Integration gateway does not include test modules")
    discovered = {}
    compiled_fixtures = set()
    for source, module in modules:
        path = repo_file("tests/" + source)
        relative = path.relative_to(REPO).as_posix()
        text = path.read_text()
        require("#[ignore" not in text, f"Ignored adversarial test in {relative}")
        for name in re.findall(r'#\[test\]\s*fn\s+(\w+)\s*\(', text):
            discovered[f"{module}::{name}"] = relative
        for fixture in re.findall(r'include_str!\(\s*"([^"]+)"\s*\)', text):
            compiled_fixtures.add((path.parent / fixture).relative_to(REPO).as_posix())
    require(set(discovered) == set(tests),
            f"Manifest/test mismatch; uncatalogued={sorted(set(discovered) - set(tests))}; "
            f"missing={sorted(set(tests) - set(discovered))}")
    for case in cases:
        require(discovered[case["test"]] == case["source"], f"Wrong source for {case['id']}")
        require(case["sequence"] in range(1, 6), f"Wrong implementation sequence for {case['id']}")
        require(bool(case["expectation"]), f"Missing expectation for {case['id']}")
        for fixture in case["fixtures"]:
            repo_file(fixture)
    inventory = {item["path"]: item["role"] for item in manifest["fixture_inventory"]}
    require(len(inventory) == len(manifest["fixture_inventory"]), "Duplicate fixture inventory paths")
    actual_files = {
        p.relative_to(REPO).as_posix()
        for p in (REPO / "tests/adversarial").rglob("*")
        if p.is_file() and ("fixtures" in p.parts or "corpus" in p.parts)
    }
    require(set(inventory) == actual_files, "Native fixture inventory does not match files")
    require(compiled_fixtures <= set(inventory), "Compiled fixture absent from inventory")
    for path, role in inventory.items():
        require(role == ("compiled-fixture" if path in compiled_fixtures else "illustrative-or-follow-up"),
                f"Wrong fixture role for {path}")
    follow_ups = manifest["follow_ups"]
    require(len({item['id'] for item in follow_ups}) == len(follow_ups), "Duplicate follow-up IDs")
    for item in follow_ups:
        require(bool(item["procedure"]), f"Missing procedure for {item['id']}")
        for fixture in item["fixtures"]:
            repo_file(fixture)
    baseline_tests = {item["test"] for item in baseline["results"]}
    require(baseline_tests <= set(tests), "Baseline test was removed or renamed")
    require(len(baseline_tests) == manifest["baseline_case_count"], "Incomplete original baseline")
    require(baseline["revision"] == manifest["baseline_revision"], "Baseline revision mismatch")
    for report in [baseline, results]:
        if report["status"] == "pending":
            require(not report["results"], "Pending report contains claimed results")
            continue
        require(report["status"] == "completed", "Invalid result status")
        rows = report["results"]
        require(len({row['test'] for row in rows}) == len(rows), "Duplicate measured result")
        require(all(row['status'] in {"passed", "failed"} for row in rows), "Skipped or invalid result")
        require(report["test_count"] == len(rows), "Result count mismatch")
        for status in ["passed", "failed"]:
            require(report[status] == sum(row["status"] == status for row in rows), "Outcome count mismatch")
        require(report["ignored"] == 0, "Adversarial report must not ignore tests")
    if results["status"] == "completed":
        require({row['test'] for row in results['results']} == set(tests), "Incomplete implementation result set")


def render_checklist(manifest: dict, baseline: dict, results: dict) -> str:
    original = {item["test"]: item["status"] for item in baseline["results"]}
    current = {item["test"]: item["status"] for item in results["results"]}
    lines = [
        "# Adversarial fixture checklist", "",
        "Generated from `cases.json`, `baseline.json`, and `results.json` by `check.py --render-checklist`.", "",
        f"Baseline `{baseline['revision']}`: **{baseline['test_count']} executed, "
        f"{baseline['passed']} passed, {baseline['failed']} failed, {baseline['ignored']} ignored.** "
        "These are overlapping desired-behavior assertions, not independent defect counts.", "",
    ]
    if results["status"] == "pending":
        lines += ["Implementation verification: **pending**. An unchecked item has no recorded passing result.", ""]
    else:
        lines += [
            f"Implementation verification: **{results['test_count']} executed, {results['passed']} passed, "
            f"{results['failed']} failed, {results['ignored']} ignored.** "
            f"Revision `{results['revision']}`; recorded source-tree hash `{results['source_tree_sha256']}`.", "",
        ]
    for sequence, label in [
        (1, "False target identities"), (2, "Common syntax and source evidence"),
        (3, "Incremental equivalence and discovery"), (4, "Observable retrieval completeness"),
        (5, "Richer type and framework capabilities"),
    ]:
        lines += [f"## {sequence}. {label}", ""]
        for case in manifest["cases"]:
            if case["sequence"] != sequence:
                continue
            now = current.get(case["test"], "pending")
            checked = "x" if now == "passed" else " "
            lines += [
                f"- [{checked}] **{case['id']}** `{case['test']}` — {case['expectation']} "
                f"Layer: {case['layer']}. Baseline: {original.get(case['test'], 'not run')}. Current: **{now}**. "
                f"[Test source](../../../{case['source']}).",
            ]
        lines.append("")
    lines += [
        "## Prepared and proposed follow-ups", "",
        "These entries are separate from the executed suite. A prepared source fixture is not an executed assertion. "
        "Verification stays pending until a named test and its measured result are added.", "",
    ]
    for item in manifest["follow_ups"]:
        checked = "x" if item["verification"] == "verified" else " "
        lines += [
            f"- [{checked}] **{item['id']} · {item['slug']}** ({item['status']}; sequence {item['sequence']}). "
            f"{item['expectation']} {item['procedure']} Verification: **{item['verification']}**.",
        ]
    lines += ["", "## Native source files", "", "| Source | Role |", "| --- | --- |"]
    for item in manifest["fixture_inventory"]:
        lines.append(f"| [{item['path']}](../../../{item['path']}) | {item['role']} |")
    lines += [
        "", "Inline synthetic source, historical timestamps, generated long lines, and synthetic graph vertices "
        "are defined in the seven Rust modules and covered by the case inventory above.", "",
    ]
    return "\n".join(lines)


def parse_log(text: str, expected_tests: set[str]) -> dict[str, str]:
    outcomes = {}
    pending = None
    for line in text.splitlines():
        start = re.match(r"test (\w+::\w+) \.\.\.\s*(.*)$", line)
        if start:
            require(pending is None, f"Missing outcome for {pending}")
            pending, tail = start.groups()
            require(pending in expected_tests, f"Unexpected test in log: {pending}")
            require(pending not in outcomes, f"Repeated test in log: {pending}")
            if tail in {"ok", "FAILED", "ignored"}:
                line = tail
        if pending is not None and line in {"ok", "FAILED", "ignored"}:
            require(line != "ignored", f"Ignored test in log: {pending}")
            outcomes[pending] = "passed" if line == "ok" else "failed"
            pending = None
    require(pending is None, f"Missing final outcome for {pending}")
    require(set(outcomes) == expected_tests, f"Incomplete log: missing {sorted(expected_tests - set(outcomes))}")
    summaries = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out", text)
    expected = (sum(value == "passed" for value in outcomes.values()),
                sum(value == "failed" for value in outcomes.values()), 0, 0, 0)
    require(len(summaries) == 1 and tuple(map(int, summaries[0])) == expected,
            "Expected one complete, unfiltered target summary matching individual results")
    return outcomes


def record_log(path: Path, manifest: dict, revision: str) -> dict:
    outcomes = parse_log(path.read_text(encoding="utf-8"), {case['test'] for case in manifest['cases']})
    source_files = sorted({
        *[p for p in (REPO / "src").rglob("*") if p.is_file()],
        *[p for p in (REPO / "tests/adversarial").rglob("*") if p.is_file()],
        REPO / "tests/adversarial_regressions.rs", REPO / "Cargo.toml", REPO / "Cargo.lock",
    })
    source_hashes = {p.relative_to(REPO).as_posix(): sha256(p) for p in source_files}
    tree_hash = hashlib.sha256(json.dumps(source_hashes, sort_keys=True).encode()).hexdigest()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO, text=True).strip())
    return {
        "schema_version": 1, "target": manifest["target"], "revision": revision,
        "worktree_dirty": dirty, "status": "completed", "test_count": len(outcomes),
        "passed": sum(value == "passed" for value in outcomes.values()),
        "failed": sum(value == "failed" for value in outcomes.values()), "ignored": 0,
        "log_sha256": sha256(path), "source_tree_sha256": tree_hash,
        "test_source_sha256": {case["source"]: sha256(repo_file(case["source"])) for case in manifest["cases"]},
        "results": [{"id": case["id"], "test": case["test"], "status": outcomes[case["test"]]} for case in manifest["cases"]],
        "note": "No ignored or filtered cases. Source-tree hash identifies the source snapshot, including any uncommitted implementation changes. Native fixture syntax is parsed, not executed as an application.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate coverage and generated checklist (default)")
    parser.add_argument("--render-checklist", action="store_true", help="regenerate CHECKLIST.md from recorded facts")
    parser.add_argument("--record-log", type=Path, help="record one completed adversarial_regressions Cargo log")
    parser.add_argument("--revision", help="full git commit of the tested base; dirty worktree is recorded separately")
    args = parser.parse_args()
    try:
        manifest, baseline, results = map(read_json, [CASE_PATH, BASELINE_PATH, RESULTS_PATH])
        if args.record_log:
            require(bool(args.revision and re.fullmatch(r"[0-9a-f]{40}", args.revision)), "--record-log requires --revision with a full git SHA")
            results = record_log(args.record_log, manifest, args.revision)
        validate(manifest, baseline, results)
        rendered = render_checklist(manifest, baseline, results)
        if args.record_log:
            RESULTS_PATH.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
        if args.render_checklist or args.record_log:
            CHECKLIST_PATH.write_text(rendered, encoding="utf-8")
        else:
            require(CHECKLIST_PATH.is_file() and CHECKLIST_PATH.read_text() == rendered,
                    "CHECKLIST.md is stale; run check.py --render-checklist")
        print(f"Validated {len(manifest['cases'])} executable cases, {len(manifest['follow_ups'])} separate follow-ups, "
              f"and {len(manifest['fixture_inventory'])} native fixture files; implementation results {results['status']}.")
        return 0
    except (KeyError, OSError, ValueError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"Fixture check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

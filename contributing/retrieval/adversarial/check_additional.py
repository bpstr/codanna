#!/usr/bin/env python3
"""Validate added/revised fixtures or record them from complete Cargo logs.

Accepts several complete test logs, including full cargo test output, and Rust
test names with any number of module segments. No tests or models are started.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

from check import HERE, REPO, read_json, repo_file, require, sha256


MANIFEST = HERE / "additional-fixtures.json"
RESULTS = HERE / "additional-results.json"
CHECKLIST = HERE / "ADDITIONAL-CHECKLIST.md"


def key(case: dict) -> tuple[str, str]:
    return case["target"], case["test"]


def validate(manifest: dict, results: dict) -> None:
    cases = manifest["cases"]
    require(len({case['id'] for case in cases}) == len(cases), "Duplicate additional case IDs")
    require(len({key(case) for case in cases}) == len(cases), "Duplicate additional test names")
    for case in cases:
        source = repo_file(case["source"]).read_text()
        function = case["test"].rsplit("::", 1)[-1]
        pattern = r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+' + re.escape(function) + r'\b'
        require(re.search(pattern, source) is not None, f"Missing executable test {case['id']}: {function}")
        require(bool(case["scenario"]), f"Missing scenario for {case['id']}")
        require(case["baseline"]["status"] in {"passed", "failed", "not_run"}, "Invalid baseline status")
        change_kind = case.get("change_kind", "new-test")
        require(change_kind in {"new-test", "revised-existing-contract", "revised-existing-fixture"}, "Invalid fixture change kind")
        if change_kind != "new-test":
            require(bool(case.get("previous_test")) and bool(case.get("revision_note")),
                    f"Revised fixture lacks its previous identity or explanation: {case['id']}")
            require(case["previous_test"] != case["test"], f"Revised fixture must identify its renamed test: {case['id']}")
        if case["target"] != "lib":
            repo_file(f"tests/{case['target']}.rs")
    doc_cases = [case for case in cases if case['baseline']['revision'] == manifest['review_base']]
    baseline = manifest["document_baseline"]
    require(len(doc_cases) == baseline['test_count'], "Measured document baseline inventory is incomplete")
    for status in ['passed', 'failed']:
        require(sum(case['baseline']['status'] == status for case in doc_cases) == baseline[status], "Document baseline count mismatch")
    require(baseline['ignored'] == 0, "Unexpected ignored baseline cases")
    if results['status'] == 'pending':
        require(not results['results'], "Pending report cannot claim measured results")
        return
    require(results['status'] == 'completed', "Invalid implementation result status")
    require({key(row) for row in results['results']} == {key(case) for case in cases}, "Incomplete measured additional suite")
    require(len(results['results']) == len(cases), "Repeated measured additional results")
    require(all(row['status'] in {'passed', 'failed'} for row in results['results']), "Ignored or invalid additional result")
    for status in ['passed', 'failed']:
        require(results[status] == sum(row['status'] == status for row in results['results']), "Additional outcome count mismatch")


def render(manifest: dict, results: dict) -> str:
    observed = {key(row): row['status'] for row in results['results']}
    new_count = sum(case.get('change_kind', 'new-test') == 'new-test' for case in manifest['cases'])
    revised_count = len(manifest['cases']) - new_count
    lines = [
        '# Additional and revised implementation fixtures', '',
        'Generated from `additional-fixtures.json` and `additional-results.json`. '
        'These cases are separate from the original 62-case investigation suite.', '',
        f"**{len(manifest['cases'])} implementation fixtures** are catalogued: "
        f"**{new_count} new tests and {revised_count} revised existing tests**. "
        'The 16 document baseline probes executed against the review commit: '
        '**1 passed, 15 failed, 0 ignored**. The other fixtures have no measured '
        'pre-change baseline for their current assertions; their existence or revision '
        'does not establish a historical failure.', '',
    ]
    if results['status'] == 'pending':
        lines += ['Final implementation verification: **pending**. Earlier work-in-progress runs are not promoted to a final source snapshot.', '']
    else:
        lines += [
            f"Final implementation verification: **{results['passed']} passed, {results['failed']} failed, 0 ignored**. "
            f"Base revision `{results['revision']}`; source-tree hash `{results['source_tree_sha256']}`.", '',
        ]
    groups = list(dict.fromkeys(case['group'] for case in manifest['cases']))
    for group in groups:
        lines += [f'## {group}', '']
        for case in manifest['cases']:
            if case['group'] != group:
                continue
            status = observed.get(key(case), 'pending')
            checked = 'x' if status == 'passed' else ' '
            baseline = case['baseline']['status'].replace('_', ' ')
            history = ''
            if case.get('change_kind', 'new-test') != 'new-test':
                history = f" Revised from `{case['previous_test']}`: {case['revision_note']}"
            lines.append(
                f"- [{checked}] **{case['id']}** `{case['target']} / {case['test']}` — "
                f"{case['scenario']} Scope: {case['scope']}. Baseline: {baseline}. "
                f"Current: **{status}**.{history} [Source](../../../{case['source']})."
            )
        lines.append('')
    lines += [
        '## Record and verify', '',
        'Use complete captured Cargo logs from the final source snapshot. Multiple logs may be '
        'supplied for separately run integration targets and unit tests. Every catalogued test must '
        'have an explicit outcome within a completed test-run summary; missing or ignored cases '
        'cannot become passes.', '',
        '```bash',
        'python3 contributing/retrieval/adversarial/check_additional.py --check',
        'python3 contributing/retrieval/adversarial/check_additional.py \\',
        '  --record-log /absolute/path/to/integration.log \\',
        '  --record-log /absolute/path/to/unit.log \\',
        '  --revision "$(git rev-parse HEAD)"',
        '```', '',
        'The recorder accepts full Cargo output and qualified unit-test names. It records only '
        'catalogued cases; unrelated suite failures still need to be assessed and reported by the '
        'repository gates. Use one Rust test thread when capturing `--nocapture` output so individual '
        'test completion lines remain attributable.', '',
    ]
    return '\n'.join(lines)


def target_from_running(line: str, targets: set[str]) -> str | None:
    if re.search(r'Running unittests src/lib\.rs\b', line):
        return 'lib'
    match = re.search(r'Running tests/([^ /]+)\.rs\b', line)
    if match:
        return match.group(1)
    # Verbose Cargo prints the executable invocation instead of its source.
    match = re.search(r'Running .*[/\\]([^ /\\`]+)-[0-9a-f]{8,}(?:[ .`]|$)', line)
    if match and match.group(1) in targets:
        return match.group(1)
    return None


def measured_rows(path: Path, cases: list[dict]) -> dict[tuple[str, str], str]:
    wanted = {key(case) for case in cases}
    by_name: dict[str, list[tuple[str, str]]] = {}
    for case in cases:
        by_name.setdefault(case['test'], []).append(key(case))
    targets = {case['target'] for case in cases}
    target = None
    pending = None
    run_rows = {}
    run_count = 0
    run_statuses = {}
    outcomes = {}
    in_run = False
    for raw_line in path.read_text(encoding='utf-8').splitlines():
        line = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', raw_line)
        if 'Running ' in line:
            if not in_run:
                target = target_from_running(line, targets)
        run_start = re.fullmatch(r'running (\d+) tests?', line)
        if run_start:
            require(not in_run, f'Incomplete prior test group in {path.name}')
            in_run = True
            run_rows = {}
            run_count = int(run_start.group(1))
            run_statuses = {}
        start = re.match(r'test (.+?) \.\.\.\s*(.*)$', line)
        if start and in_run:
            require(pending is None, f'Interleaved or incomplete --nocapture output in {path.name}; use --test-threads=1')
            name, tail = start.groups()
            selected = (target, name) if target is not None and (target, name) in wanted else None
            if target is None and name in by_name:
                require(len(by_name[name]) == 1, f'Ambiguous test target for {name}')
                selected = by_name[name][0]
            pending = selected, name
            if tail in {'ok', 'FAILED', 'ignored'} or tail.startswith('ignored,'):
                line = tail
        if pending is not None and (line in {'ok', 'FAILED', 'ignored'} or line.startswith('ignored,')):
            selected, name = pending
            require(name not in run_statuses, f'Duplicate test within group: {name}')
            run_statuses[name] = 'ignored' if line.startswith('ignored') else 'passed' if line == 'ok' else 'failed'
            if selected is not None:
                require(not line.startswith('ignored'), f'Catalogued test was ignored: {name}')
                require(selected not in run_rows, f'Duplicate test within group: {name}')
                run_rows[selected] = 'passed' if line == 'ok' else 'failed'
            pending = None
        summary = re.match(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', line)
        if summary:
            require(in_run, f'Test summary without a running group in {path.name}')
            require(pending is None, f'Missing outcome before summary in {path.name}')
            require(len(run_statuses) == run_count, f'Incomplete test output in {path.name}: expected {run_count}, observed {len(run_statuses)}')
            for status, count in zip(('passed', 'failed', 'ignored'), summary.groups()):
                require(sum(value == status for value in run_statuses.values()) == int(count),
                        f'Test summary disagrees with {status} outcomes in {path.name}')
            for selected, status in run_rows.items():
                require(selected not in outcomes, f'Test repeated in one log: {selected}')
                outcomes[selected] = status
            run_rows = {}
            in_run = False
    require(not in_run and pending is None, f'Test run did not finish in {path.name}')
    return outcomes


def record(paths: list[Path], manifest: dict, revision: str) -> dict:
    outcomes = {}
    for path in paths:
        for selected, status in measured_rows(path, manifest['cases']).items():
            require(selected not in outcomes, f'Test repeated across logs: {selected}; supply only its final run')
            outcomes[selected] = status
    wanted = {key(case) for case in manifest['cases']}
    require(set(outcomes) == wanted, f'Missing measured cases: {sorted(wanted - set(outcomes))}')
    sources = sorted({case['source'] for case in manifest['cases']})
    source_paths = sorted({
        *[p for p in (REPO / 'src').rglob('*') if p.is_file()],
        *[p for p in (REPO / 'tests').rglob('*') if p.is_file()],
        REPO / 'Cargo.toml', REPO / 'Cargo.lock',
    })
    hashes = {p.relative_to(REPO).as_posix(): sha256(p) for p in source_paths}
    return {
        'schema_version': 1, 'status': 'completed', 'revision': revision,
        'worktree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=REPO, text=True).strip()),
        'source_tree_sha256': hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest(),
        'log_sha256': [sha256(path) for path in paths],
        'test_source_sha256': {source: sha256(repo_file(source)) for source in sources},
        'passed': sum(status == 'passed' for status in outcomes.values()),
        'failed': sum(status == 'failed' for status in outcomes.values()),
        'ignored': 0,
        'results': [
            {'id': case['id'], 'target': case['target'], 'test': case['test'], 'status': outcomes[key(case)]}
            for case in manifest['cases']
        ],
        'note': 'All listed cases have measured outcomes from completed Cargo groups. No outcome for uncatalogued repository tests is implied.',
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='validate sources and generated checklist (default)')
    parser.add_argument('--render-checklist', action='store_true')
    parser.add_argument('--record-log', action='append', type=Path, default=[])
    parser.add_argument('--revision')
    args = parser.parse_args()
    try:
        manifest, results = read_json(MANIFEST), read_json(RESULTS)
        if args.record_log:
            require(bool(args.revision and re.fullmatch(r'[0-9a-f]{40}', args.revision)), '--record-log requires a full --revision SHA')
            results = record(args.record_log, manifest, args.revision)
        validate(manifest, results)
        checklist = render(manifest, results)
        if args.record_log:
            RESULTS.write_text(json.dumps(results, indent=2) + '\n')
        if args.record_log or args.render_checklist:
            CHECKLIST.write_text(checklist)
        else:
            require(CHECKLIST.is_file() and CHECKLIST.read_text() == checklist,
                    'ADDITIONAL-CHECKLIST.md is stale; run check_additional.py --render-checklist')
        print(f"Validated {len(manifest['cases'])} additional/revised executable fixtures; final results {results['status']}.")
        return 0
    except (KeyError, OSError, ValueError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f'Additional fixture check failed: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Local retrieval acceptance checks. Python 3.11+, standard library only."""
from __future__ import annotations

import argparse
import ast
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any

HERE = Path(__file__).resolve().parent
MAX_OUTPUT = 16 * 1024 * 1024


class ScopeError(ValueError):
    """Retrieved evidence crossed a workspace or exclusion boundary."""


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def corpus_digest(root: Path) -> str:
    h = hashlib.sha256()
    for path in sorted(p for p in root.rglob('*') if p.is_file()
                       and '__pycache__' not in p.parts and p.suffix != '.pyc'):
        for value in (path.relative_to(root).as_posix().encode(), path.read_bytes()):
            h.update(len(value).to_bytes(8, 'little'))
            h.update(value)
    return h.hexdigest()


def source_path(root: Path, value: str) -> Path:
    path = (root / value).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ScopeError(f'Out-of-scope evidence path: {value}')
    if not path.is_file():
        raise ValueError(f'Invalid evidence path: {value}')
    return path


def validate_manifest(manifest: dict, root: Path = HERE) -> None:
    if manifest.get('schema_version') != 1:
        raise ValueError('Unsupported case schema')
    seen: set[str] = set()
    for case in manifest['cases'] + manifest['manual_cases']:
        if case['id'] in seen:
            raise ValueError(f'Duplicate case ID: {case["id"]}')
        seen.add(case['id'])
        if not case.get('title'):
            raise ValueError('Every case needs a title')
        for path in case.get('paths', []):
            source_path(root / 'workspace', path)
        if 'kind' not in case:
            if not case.get('requirement'):
                raise ValueError('Manual case has no requirement')
            continue
        if case['kind'] not in {'graph', 'context', 'documents', 'semantic_code'}:
            raise ValueError(f'Unknown case kind: {case["kind"]}')
        if case['tier'] not in {'invariant', 'target'}:
            raise ValueError('Unknown tier')
        if not case['profiles'] or set(case['profiles']) - {'structural', 'lexical', 'semantic'}:
            raise ValueError('Unknown or empty profile')
        if case['kind'] in {'documents', 'semantic_code'}:
            if case['k'] != 5 or not case['query']:
                raise ValueError('Retrieval cases must have a query and k=5')
            if not case['required'] and not case.get('empty'):
                raise ValueError('Retrieval case has no positive or negative oracle')
        def walk(value: Any) -> None:
            if isinstance(value, dict):
                if 'path' in value:
                    path = source_path(root / 'workspace', value['path'])
                    if 'contains' in value and value['contains'] not in path.read_text(encoding='utf-8'):
                        raise ValueError(f'Gold text missing from {path}')
                    if 'max_rank' in value and not 1 <= value['max_rank'] <= 5:
                        raise ValueError('Invalid maximum rank')
                for nested in value.values():
                    walk(nested)
            elif isinstance(value, list):
                for nested in value:
                    walk(nested)
        walk(case)
    for base in ('workspace', 'peer-workspace'):
        for path in (root / base).rglob('*'):
            if path.is_symlink():
                raise ValueError('Committed fixtures must not contain symlinks')
            if path.suffix == '.py':
                ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    long_doc = (root / 'workspace/docs/long-runbook.md').read_text(encoding='utf-8')
    if long_doc.index('violet-heron') < 2000:
        raise ValueError('Long-document witness is no longer beyond the prefix')


def matches(node: dict, selector: dict) -> bool:
    source = node['source']
    return (source['path'] == selector['path']
            and ('kind' not in selector or node['kind'] == selector['kind'])
            and ('name' not in selector or node['label'].rsplit('::', 1)[-1] == selector['name'])
            and ('label' not in selector or node['label'] == selector['label'])
            and ('start_line' not in selector or source['start_line'] == selector['start_line']))


def selected(graph: dict, selector: dict) -> set[str]:
    return {key for key, node in graph['nodes'].items() if matches(node, selector)}


def verify_span(span: dict, root: Path, repo: str) -> None:
    if span['repo'] != repo:
        raise ScopeError('Cross-workspace evidence')
    path = source_path(root, span['path'])
    if not 1 <= span['start_line'] <= span['end_line'] <= max(1, len(path.read_text(encoding='utf-8').splitlines())):
        raise ValueError(f'Invalid source span: {span}')
    if span['hash'] != digest(path):
        raise ValueError(f'Stale source hash: {span["path"]}')


def verify_graph(graph: dict, root: Path, repo: str) -> None:
    if graph['schema_version'] != 1 or set(graph['repositories']) != {repo}:
        raise ValueError('Unexpected graph schema or repository scope')
    for key, node in graph['nodes'].items():
        if key != node['id']:
            raise ValueError('Node identity mismatch')
        verify_span(node['source'], root, repo)
    for edge in graph['edges']:
        if edge['from'] not in graph['nodes'] or edge['to'] not in graph['nodes']:
            raise ValueError('Dangling edge')
        verify_span(edge['evidence'], root, repo)
    for row in graph['unresolved']:
        if row['from'] not in graph['nodes'] or set(row['candidates']) - graph['nodes'].keys():
            raise ValueError('Dangling unresolved reference')
        verify_span(row['evidence'], root, repo)


def graph_check(check: dict, graph: dict, peer: dict) -> bool:
    op = check['op']
    if op == 'node':
        return bool(selected(graph, check['selector']))
    if op in {'edge', 'no_edge'}:
        sources = selected(graph, check['source'])
        targets = selected(graph, check['target']) if 'target' in check else set(graph['nodes'])
        found = any(e['from'] in sources and e['to'] in targets
                    and all(e.get(k) == check[k] for k in ('relation', 'basis', 'method') if k in check)
                    for e in graph['edges'])
        return found if op == 'edge' else bool(sources) and not found
    if op in {'unresolved', 'no_unresolved'}:
        rows = [r for r in graph['unresolved']
                if graph['nodes'][r['from']]['source']['path'] == check['path']
                and r['reference'] == check['reference']]
        if op == 'no_unresolved':
            return bool(selected(graph, {'path': check['path']})) and not rows
        return any(('reason' not in check or r['reason'] == check['reason'])
                   and ('candidates' not in check or len(r['candidates']) == check['candidates'])
                   for r in rows)
    if op == 'no_path':
        return not any(n['source']['path'].startswith(check['prefix']) for n in graph['nodes'].values())
    if op == 'peer_isolation':
        selector = {'path': 'src/storage.py', 'name': 'admit_blob', 'kind': 'symbol'}
        text, other = json.dumps(graph), json.dumps(peer)
        return (bool(selected(graph, selector)) and bool(selected(peer, selector))
                and 'PEER_ONLY_COBALT_62' not in text and 'PEER_ONLY_COBALT_62' in other
                and not set(graph['nodes']).intersection(peer['nodes']))
    raise ValueError(f'Unknown graph check: {op}')


def envelope_items(envelope: dict, returncode: int) -> list[dict]:
    if (envelope.get('status') == 'not_found' and envelope.get('code') == 'NOT_FOUND'
            and returncode in (0, 1) and envelope.get('data') in (None, [], {'items': []})):
        return []
    if returncode != 0 or envelope.get('status') != 'success' or envelope.get('code') != 'OK':
        raise ValueError(f'Retrieval error, not an empty result: {envelope.get("code")}')
    data = envelope.get('data')
    if isinstance(data, dict):
        data = data.get('items')
    if not isinstance(data, list) or any(not isinstance(row, dict) for row in data):
        raise ValueError('Unexpected retrieval envelope; inspect raw output')
    return data


def normalize_rows(items: list[dict], kind: str, workspace: Path) -> list[dict]:
    rows = []
    for item in items:
        if kind == 'documents':
            path = source_path(workspace, item['source_path'])
            start, end = item['byte_range']
            content = path.read_bytes()
            if not 0 <= start < end <= len(content):
                raise ValueError('Invalid document byte range')
            text = content[start:end].decode('utf-8')
            name = None
        else:
            symbol = item['symbol']
            path = source_path(workspace, symbol['file_path'])
            start, end = symbol['range']['start_line'], symbol['range']['end_line']
            lines = path.read_text(encoding='utf-8').splitlines()
            if not 0 <= start <= end < len(lines):
                raise ValueError('Invalid code range')
            text, name = '\n'.join(lines[start:end + 1]), symbol['name']
        relative = path.relative_to(workspace.resolve()).as_posix()
        if relative.startswith(('docs/private/', 'src/generated/')):
            raise ScopeError('Excluded evidence was retrieved')
        rows.append({'path': relative, 'name': name, 'text': text})
    return rows


def score_retrieval(case: dict, rows: list[dict]) -> dict:
    rows = rows[:case['k']]
    ranks = []
    for gold in case['required']:
        ranks.append(next((i for i, row in enumerate(rows, 1)
                           if row['path'] == gold['path']
                           and ('name' not in gold or row['name'] == gold['name'])
                           and ('contains' not in gold or gold['contains'] in row['text'])), None))
    passed = all(rank is not None and rank <= gold.get('max_rank', case['k'])
                 for rank, gold in zip(ranks, case['required']))
    counts = Counter(row['path'] for row in rows)
    forbidden = bool(rows and rows[0]['path'] in case.get('forbidden_first', []))
    passed = (passed and not forbidden and (not case.get('empty') or not rows)
              and max(counts.values(), default=0) <= case.get('max_per_path', case['k'])
              and len(counts) >= case.get('min_distinct_paths', 0))
    found = [rank for rank in ranks if rank is not None]
    return {'passed': passed, 'ranks': ranks, 'paths': [row['path'] for row in rows],
            'hit_at_5': int(bool(found)) if ranks else None,
            'reciprocal_rank_at_5': 1 / min(found) if found else (0 if ranks else None),
            'required_evidence_recall_at_5': len(found) / len(ranks) if ranks else None,
            'forbidden_evidence': int(forbidden)}


def isolated_env(home: Path) -> dict[str, str]:
    # Allowlist, never copy provider credentials, CI_ overrides, dotenv or user configs.
    env = {key: os.environ[key] for key in ('PATH', 'SYSTEMROOT', 'WINDIR') if key in os.environ}
    env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
               XDG_CACHE_HOME=str(home / 'cache'), HF_HOME=str(home / 'huggingface'),
               TMPDIR=str(home / 'tmp'), TEMP=str(home / 'tmp'), TMP=str(home / 'tmp'),
               NO_COLOR='1', OMP_NUM_THREADS='2', RAYON_NUM_THREADS='2')
    return env


class Commands:
    def __init__(self, out: Path, env: dict[str, str]):
        self.out, self.env, self.history = out, env, []
        (out / 'raw').mkdir()

    def call(self, argv: list[str], cwd: Path, timeout: float = 60, allowed=(0,)) -> tuple[bytes, int]:
        index = len(self.history)
        stdout, stderr = self.out / 'raw' / f'{index:03d}.out', self.out / 'raw' / f'{index:03d}.err'
        entry = {'argv': argv, 'cwd': str(cwd), 'stdout': str(stdout.relative_to(self.out)),
                 'stderr': str(stderr.relative_to(self.out))}
        self.history.append(entry)
        start = time.monotonic()
        with stdout.open('wb') as out, stderr.open('wb') as err:
            process = subprocess.Popen(argv, cwd=cwd, env=self.env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err, start_new_session=os.name == 'posix')
            try:
                returncode = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                if os.name == 'posix':
                    os.killpg(process.pid, signal.SIGKILL)
                else:
                    process.kill()
                process.wait()
                entry.update(error='timeout', elapsed_seconds=time.monotonic() - start)
                raise RuntimeError(f'Command timed out; inspect {stderr}') from None
        entry.update(returncode=returncode, elapsed_seconds=time.monotonic() - start)
        if stdout.stat().st_size > MAX_OUTPUT or stderr.stat().st_size > MAX_OUTPUT:
            raise RuntimeError('Command output exceeded the bounded fixture budget')
        if returncode not in allowed:
            raise RuntimeError(f'Command exited {returncode}; inspect {stderr}')
        return stdout.read_bytes(), returncode


def binary(value: str) -> str:
    resolved = shutil.which(value)
    if resolved is None:
        raise ValueError(f'Binary not found: {value}; build it explicitly first')
    return str(Path(resolved).resolve())


def write_settings(workspace: Path, semantic: bool, code_representation: str = 'doc_comment') -> Path:
    config = workspace / '.codanna/settings.toml'
    config.parent.mkdir(exist_ok=True)
    config.write_text(f'''version = 1
workspace_root = {json.dumps(str(workspace))}
index_path = {json.dumps(str(workspace / '.codanna/index'))}

[indexing]
parallelism = 2
show_progress = false

[semantic_search]
enabled = {str(semantic).lower()}
model = "AllMiniLML6V2"
code_representation = {json.dumps(code_representation)}
embedding_threads = 2

[documents]
enabled = true

[documents.collections.docs]
paths = ["docs"]
patterns = ["**/*.md", "**/*.txt"]
''', encoding='utf-8')
    return config


def run_case(case: dict, graph: dict, peer: dict, cmd: Commands,
             codanna: str, knowledge: str, workspace: Path, config: Path) -> dict:
    kind = case['kind']
    if kind == 'graph':
        checks = [graph_check(check, graph, peer) for check in case['checks']]
        return {'passed': all(checks), 'checks': checks,
                'forbidden_evidence': sum(not ok for check, ok in zip(case['checks'], checks)
                                          if check['op'] in ('no_path', 'peer_isolation'))}
    if kind == 'context':
        argv = [knowledge, 'context', '--graph', str(workspace / '.codanna/knowledge.json'),
                '--repo', 'primary', '--max-bytes', str(case['max_bytes']),
                '--max-nodes', str(case['max_nodes']), '--max-depth', str(case['max_depth'])]
        if 'seed' in case:
            ids = selected(graph, case['seed'])
            if len(ids) != 1:
                raise ValueError(f'Expected one indexed seed, got {len(ids)}')
            argv += ['--entity', next(iter(ids))]
        for path in case.get('files', []):
            argv += ['--file', path]
        argv += [case.get('query', '')]
        raw, _ = cmd.call(argv, workspace)
        bundle = json.loads(raw)
        nodes = {item['node']['id']: item['node'] for item in bundle['items']}
        if len(nodes) != len(bundle['items']) or bundle['returned_nodes'] != len(nodes):
            raise ValueError('Duplicate or inconsistent context nodes')
        verify_graph({'schema_version': bundle['schema_version'], 'repositories': {'primary': {}},
                      'nodes': nodes, 'edges': bundle['edges'], 'unresolved': bundle['unresolved']},
                     workspace, 'primary')
        checks = [any(matches(node, expected) for node in nodes.values()) for expected in case['required']]
        passed = (all(checks) and len(raw.rstrip(b'\n')) <= case['max_bytes']
                  and len(nodes) <= case['max_nodes'] and bundle['scope'] == 'primary'
                  and bundle['freshness'] == 'indexed_snapshot_not_live_source'
                  and ('truncated' not in case or bundle['truncated'] == case['truncated'])
                  and all(edge['basis'] != 'candidate' for edge in bundle['edges']))
        return {'passed': passed, 'checks': checks, 'returned_nodes': len(nodes),
                'payload_bytes': len(raw.rstrip(b'\n')), 'truncated': bundle['truncated']}
    if kind == 'documents':
        argv = [codanna, '--config', str(config), 'documents', 'search', case['query'],
                '--collection', case.get('collection', 'docs'), '--limit', str(case['k']), '--json']
    else:
        argv = [codanna, '--config', str(config), 'mcp', 'semantic_search_docs',
                'query:' + case['query'], 'limit:' + str(case['k']), '--json']
    raw, returncode = cmd.call(argv, workspace, allowed=(0, 1, 2))
    items = envelope_items(json.loads(raw), returncode)
    return score_retrieval(case, normalize_rows(items, kind, workspace))


def summarize(results: list[dict], minimums: dict) -> dict:
    positive = [r for r in results if r.get('hit_at_5') is not None]
    def average(key: str) -> float | None:
        return sum(r[key] for r in positive) / len(positive) if positive else None
    invariants = [r for r in results if r['tier'] == 'invariant']
    metrics = {'invariant_pass_rate': sum(r['passed'] for r in invariants) / len(invariants) if invariants else None,
               'hit_at_5': average('hit_at_5'), 'mrr_at_5': average('reciprocal_rank_at_5'),
               'mean_required_evidence_recall_at_5': average('required_evidence_recall_at_5'),
               'forbidden_evidence': sum(r.get('forbidden_evidence', 0) for r in results)}
    measured = {k: v for k, v in metrics.items() if v is not None}
    floor = all(v <= minimums[k] if k == 'forbidden_evidence' else v >= minimums[k]
                for k, v in measured.items())
    return {'metrics': metrics, 'measured_minimums_met': floor,
            'selected_cases_passed': bool(results) and all(r['passed'] for r in results),
            'passed': sum(r['passed'] for r in results), 'failed': sum(not r['passed'] for r in results)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Validate fixtures and gold only; no Codanna/model calls')
    parser.add_argument('--profile', choices=('structural', 'lexical', 'semantic'), default='structural')
    parser.add_argument('--codanna', default='codanna')
    parser.add_argument('--knowledge', default='codanna-knowledge')
    parser.add_argument('--code-representation', choices=('doc_comment', 'symbol_body_v1'),
                        default='doc_comment', help='Explicit semantic code input policy; defaults to rc2 behavior')
    parser.add_argument('--out', type=Path, help='New output directory outside this checkout; never overwritten')
    args = parser.parse_args()
    manifest = json.loads((HERE / 'cases.json').read_text(encoding='utf-8'))
    validate_manifest(manifest)
    if args.check:
        print(f'Validated {len(manifest["cases"])} executable cases and {len(manifest["manual_cases"])} manual scenarios; no retrieval was run.')
        return 0
    codanna, knowledge = binary(args.codanna), binary(args.knowledge)
    if args.out:
        out = args.out.expanduser().resolve()
        if out.is_relative_to(HERE.parents[1]):
            raise ValueError('Output must be outside the checkout to avoid ancestor ignore/config leakage')
        out.mkdir(parents=True, exist_ok=False)
    else:
        out = Path(tempfile.mkdtemp(prefix='codanna-retrieval-')).resolve()
    home = out / 'home'
    for directory in (home, home / 'tmp', home / 'config', home / 'cache'):
        directory.mkdir()
    cmd = Commands(out, isolated_env(home))
    report: dict[str, Any] = {'schema_version': 1, 'profile': args.profile, 'cases': [],
        'fixture_sha256': corpus_digest(HERE / 'workspace'), 'peer_fixture_sha256': corpus_digest(HERE / 'peer-workspace'),
        'cases_sha256': digest(HERE / 'cases.json'), 'binary_sha256': digest(Path(codanna)),
        'knowledge_binary_sha256': digest(Path(knowledge)), 'manual_pending': manifest['manual_cases'],
        'full_qualification': False, 'commands': cmd.history,
        'inspected_main': manifest['inspected_main'], 'model': 'AllMiniLML6V2' if args.profile == 'semantic' else None,
        'code_representation': args.code_representation if args.profile == 'semantic' else None}
    try:
        for executable, key in ((codanna, 'codanna_version'), (knowledge, 'knowledge_version')):
            raw, _ = cmd.call([executable, '--version'], out)
            report[key] = raw.decode().strip()
        graphs, configs = {}, {}
        for repo, fixture in (('primary', 'workspace'), ('peer', 'peer-workspace')):
            workspace = out / repo
            shutil.copytree(HERE / fixture, workspace, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
            config = write_settings(workspace, args.profile == 'semantic' and repo == 'primary',
                                    args.code_representation)
            configs[repo] = config
            paths = [path for path in ('src', 'clients', 'tests') if (workspace / path).is_dir()]
            cmd.call([codanna, '--config', str(config), 'index', *paths, '--threads', '2', '--no-progress'], workspace, timeout=300)
            dump, _ = cmd.call([codanna, '--config', str(config), 'dump'], workspace)
            dump_path = workspace / '.codanna/dump.jsonl'
            dump_path.write_bytes(dump)
            cmd.call([knowledge, 'index', '--root', str(workspace), '--repo', repo, '--dump', str(dump_path)], workspace)
            graphs[repo] = json.loads((workspace / '.codanna/knowledge.json').read_text(encoding='utf-8'))
            verify_graph(graphs[repo], workspace, repo)
        workspace = out / 'primary'
        if args.profile in ('lexical', 'semantic'):
            cmd.call([codanna, '--config', str(configs['primary']), 'documents', 'index', '--collection', 'docs', '--no-progress'], workspace, timeout=300)
        selected_cases = [c for c in manifest['cases'] if 'structural' in c['profiles'] or args.profile in c['profiles']]
        report['not_selected'] = [c['id'] for c in manifest['cases'] if c not in selected_cases]
        for case in selected_cases:
            start = time.monotonic()
            try:
                result = run_case(case, graphs['primary'], graphs['peer'], cmd, codanna, knowledge, workspace, configs['primary'])
            except (ValueError, KeyError, TypeError, IndexError, RuntimeError, OSError) as error:
                result = {'passed': False, 'error': str(error),
                          'forbidden_evidence': int(isinstance(error, ScopeError))}
                if case['kind'] in ('documents', 'semantic_code') and case['required']:
                    result.update(hit_at_5=0, reciprocal_rank_at_5=0, required_evidence_recall_at_5=0)
            result.update(id=case['id'], title=case['title'], tier=case['tier'], elapsed_seconds=time.monotonic() - start)
            report['cases'].append(result)
            print(f'{"PASS" if result["passed"] else "FAIL"} {case["id"]}: {case["title"]}')
        report.update(summarize(report['cases'], manifest['minimums']))
        report['status'] = 'pass_for_selected_profile' if report['selected_cases_passed'] and report['measured_minimums_met'] else 'fail'
        return 0 if report['status'] == 'pass_for_selected_profile' else 1
    except (ValueError, KeyError, TypeError, IndexError, RuntimeError, OSError) as error:
        report.update(status='setup_error', error=str(error))
        print(f'Setup error: {error}', file=sys.stderr)
        return 2
    finally:
        (out / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        print(f'Report and raw commands: {out / "report.json"}')


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)

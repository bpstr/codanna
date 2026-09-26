#!/usr/bin/env python3
"""Index a bounded copy of Codanna source with cached local embeddings.

Never indexes the working checkout or Assign. The source-level judgments below
are development probes selected by the implementer, not independent gold labels.
Raw output, hashes, settings, and failures stay in a new directory outside Git.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import sys

# Reuse the same allowlisted environment and source-span validation as acceptance.
import run as acceptance

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ['src/documents/store.rs', 'src/documents/ranking.rs',
           'src/documents/chunker.rs', 'src/documents/config.rs',
           'src/documents/generation.rs', 'src/embedding_input.rs',
           'src/embedding_cache.rs', 'src/symbol_representation.rs']
CASES = [
    ('Partition document text into pieces that each fit the tokenizer input limit',
     'src/embedding_input.rs', 'document_ranges'),
    ('Save cached embedding vectors to disk for reuse after restarting',
     'src/embedding_cache.rs', 'save'),
    ('Choose which code symbols can be embedded even without documentation comments',
     'src/symbol_representation.rs', 'eligible'),
    ('Prevent one document from occupying every search result',
     'src/documents/ranking.rs', 'diversify'),
    ('Find document chunks restricted to the requested collection and source path',
     'src/documents/store.rs', 'get_filtered_candidates'),
    ('Build a text preview highlighting words matching the query',
     'src/documents/store.rs', 'generate_preview'),
]


def code_rows(raw, code, workspace):
    items = acceptance.envelope_items(json.loads(raw), code)
    if len(items) > 5:
        raise ValueError('CLI exceeded the top-five result budget')
    return acceptance.normalize_rows(items, 'semantic_code', workspace)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codanna', type=Path, required=True)
    parser.add_argument('--planner', type=Path, required=True)
    parser.add_argument('--cached-models', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--model', choices=('AllMiniLML6V2', 'MultilingualE5Small'), default='AllMiniLML6V2')
    parser.add_argument('--allow-local-model', action='store_true')
    args = parser.parse_args()
    if not args.allow_local_model:
        parser.error('Explicit --allow-local-model is required')
    out = args.out.resolve()
    if out.is_relative_to(ROOT):
        parser.error('Output must be outside the checkout')
    # This helper validates/copies just the five selected model assets and ref.
    import importlib.util
    spec = importlib.util.spec_from_file_location('qualification', ROOT / 'contributing/retrieval/qualify-local.py')
    qualification = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(qualification)
    evaluator = qualification.module('evaluation', ROOT / 'contributing/retrieval/embedding-followups/semantic-evaluation/evaluate.py')
    cache = args.cached_models.resolve()
    assets = evaluator.cache_inventory(cache, args.model)
    out.mkdir(parents=True, exist_ok=False)
    workspace, home = out / 'workspace', out / 'home'
    for directory in (workspace, home, home / 'tmp', home / 'config', home / 'cache'):
        directory.mkdir()
    for source in SOURCES:
        target = workspace / source
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / source, target)
    for relative in assets:
        target = home / '.codanna/models' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(cache / relative, target)
    if evaluator.inventory(home / '.codanna/models') != assets:
        raise ValueError('Cached model changed during copy')
    env = acceptance.isolated_env(home)
    env.update(HF_HOME=str(home / '.codanna/models'), HF_ENDPOINT='codanna-offline://cache')
    config = acceptance.write_settings(workspace, True, 'symbol_body_v1', args.model)
    codanna, planner = str(args.codanna.resolve()), str(args.planner.resolve())
    cmd = acceptance.Commands(out, env)
    report = {'full_qualification': False, 'model': args.model,
              'sources': evaluator.inventory(workspace / 'src'),
              'cases': CASES, 'cached_artifacts': assets, 'commands': cmd.history,
              'binary_sha256': acceptance.digest(Path(codanna)),
              'planner_sha256': acceptance.digest(Path(planner)), 'results': []}
    base = [codanna, '--config', str(config)]
    try:
        version, _ = cmd.call([codanna, '--version'], workspace)
        report['version'] = version.decode().strip()
        before = evaluator.inventory(workspace)
        raw, plan_code = cmd.call([planner, '--config', str(config), 'src'], workspace, allowed=(0, 3))
        report['preflight'] = json.loads(raw)
        report['preflight_exit_code'] = plan_code
        report['preflight_read_only'] = before == evaluator.inventory(workspace)
        if not report['preflight_read_only']:
            raise ValueError('Preflight modified the disposable workspace')
        plan = report['preflight']
        # The source-only planner deliberately does not load a local tokenizer.
        # Retain its partial status and unknown segment counts, never call it
        # complete. This explicit bounded local run may measure those inputs.
        local_unknown = (plan_code == 3 and plan['status'] == 'partial'
                         and plan['backend'] == 'local'
                         and plan['cache_lookup'] == 'local_model_identity_not_loaded'
                         and plan['files_parsed'] == plan['files_discovered'] == len(SOURCES)
                         and plan['files_requiring_generic_parser'] == 0
                         and plan['missing_body_sources'] == 0
                         and plan['input_policy_rejections'] is None)
        if not ((plan_code == 0 and plan['status'] == 'complete') or local_unknown):
            raise ValueError('Preflight has an unexpected partial or blocked inventory')
        cmd.call(base + ['index', 'src', '--threads', '2', '--no-progress'], workspace, timeout=600)
        raw, _ = cmd.call(base + ['mcp', 'get_index_info', '--json'], workspace)
        report['index_info'] = json.loads(raw)
        for text, path, name in CASES:
            raw, code = cmd.call(base + ['mcp', 'semantic_search_docs', f'query:{text}',
                                         'limit:5', '--json'], workspace, allowed=(0, 1))
            rows = code_rows(raw, code, workspace)
            rank = next((i for i, row in enumerate(rows, 1)
                         if row['path'] == path and row['name'] == name), None)
            report['results'].append({'query': text, 'expected': [path, name], 'rank': rank,
                                      'returned': [[row['path'], row['name']] for row in rows]})
        # Force rebuilding the same sample must preserve retrieval and source scope.
        cmd.call(base + ['index', 'src', '--force', '--threads', '2', '--no-progress'], workspace, timeout=600)
        report['force_rebuild_stable'] = True
        for result in report['results']:
            raw, code = cmd.call(base + ['mcp', 'semantic_search_docs', f'query:{result["query"]}',
                                         'limit:5', '--json'], workspace, allowed=(0, 1))
            rows = code_rows(raw, code, workspace)
            stable = [[row['path'], row['name']] for row in rows] == result['returned']
            result['force_rebuild_stable'] = stable
            report['force_rebuild_stable'] &= stable
        # A disposable source mutation exercises the CLI incremental lifecycle.
        # This is explicitly not a watcher or long-lived MCP freshness claim.
        probe = workspace / 'src/rc3_lifecycle_probe.rs'
        report['freshness'] = []
        for old, new in [(None, 'before_refresh'), ('before_refresh', 'after_refresh')]:
            probe.write_text(f'pub fn {new}() -> bool {{ true }}\n')
            cmd.call(base + ['index', 'src', '--threads', '2', '--no-progress'], workspace, timeout=600)
            raw, _ = cmd.call(base + ['dump'], workspace)
            names = {(row.get('data') or {}).get('name')
                     for row in map(json.loads, raw.decode().splitlines())}
            report['freshness'].append({'old': old, 'new': new,
                                        'passed': new in names and (old is None or old not in names)})
        probe.unlink()
        cmd.call(base + ['index', 'src', '--threads', '2', '--no-progress'], workspace, timeout=600)
        raw, _ = cmd.call(base + ['dump'], workspace)
        names = {(row.get('data') or {}).get('name')
                 for row in map(json.loads, raw.decode().splitlines())}
        report['freshness'].append({'deleted': 'after_refresh', 'passed': 'after_refresh' not in names})
        report['source_copy_unchanged'] = report['sources'] == evaluator.inventory(workspace / 'src')
        count = len(CASES)
        report['hit_at_5'] = sum(row['rank'] is not None for row in report['results']) / count
        report['mrr_at_5'] = sum(1 / row['rank'] if row['rank'] else 0 for row in report['results']) / count
        report['status'] = 'measured'
        return int(not report['force_rebuild_stable'] or report['hit_at_5'] < 1
                   or not report['source_copy_unchanged']
                   or not all(row['passed'] for row in report['freshness']))
    except (ValueError, OSError, RuntimeError, KeyError) as error:
        report.update(status='error', error=str(error))
        return 2
    finally:
        evaluator.write_json(out / 'report.json', report)
        print(out / 'report.json')


if __name__ == '__main__':
    sys.exit(main())

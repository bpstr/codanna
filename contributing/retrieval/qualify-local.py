#!/usr/bin/env python3
"""Bounded local qualification using an existing MiniLM snapshot; no downloads.

Outputs must be outside the checkout. Runs the unchanged acceptance queries with
both code input policies and the graded document corpus. Scores are development
measurements, not unseen or full product qualification. Python 3.11+ required.
"""
from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codanna', type=Path, required=True)
    parser.add_argument('--knowledge', type=Path, required=True)
    parser.add_argument('--cached-models', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--allow-local-model', action='store_true')
    args = parser.parse_args()
    if not args.allow_local_model:
        parser.error('Explicit --allow-local-model is required')
    out, cache = args.out.resolve(), args.cached_models.resolve()
    if out.is_relative_to(HERE.parents[1]):
        parser.error('Keep generated evidence and indexes outside the checkout')
    evaluator = module('graded_evaluation', HERE / 'embedding-followups/semantic-evaluation/evaluate.py')
    runner = module('retrieval_acceptance', HERE / 'run.py')
    assets = evaluator.cache_inventory(cache, 'AllMiniLML6V2')
    original_env = runner.isolated_env

    def offline_env(home):
        destination = home / '.codanna/models'
        for relative in assets:
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(cache / relative, target)
        if evaluator.inventory(destination) != assets:
            raise ValueError('Cached model changed during copy')
        env = original_env(home)
        env.update(HF_HOME=str(destination), HF_ENDPOINT='codanna-offline://cache')
        return env

    out.mkdir(parents=True, exist_ok=False)
    summary = {'full_qualification': False, 'cached_artifacts': assets, 'runs': {}}
    for name, profile, policy in [('lexical', 'lexical', 'doc_comment'),
                                  ('semantic-default', 'semantic', 'doc_comment'),
                                  ('semantic-body', 'semantic', 'symbol_body_v1')]:
        runner.isolated_env = offline_env if profile == 'semantic' else original_env
        sys.argv = ['run.py', '--profile', profile, '--code-representation', policy,
                    '--codanna', str(args.codanna.resolve()),
                    '--knowledge', str(args.knowledge.resolve()), '--out', str(out / name)]
        code = runner.main()
        report = evaluator.read_json(out / name / 'report.json')
        summary['runs'][name] = {'exit_code': code, 'status': report['status'],
                                'passed': report.get('passed'), 'failed': report.get('failed'),
                                'metrics': report.get('metrics')}
        evaluator.write_json(out / 'summary.json', summary)
        if code == 2:
            return 2
    revision = (cache / evaluator.LOCAL_MODELS['AllMiniLML6V2'][0] / 'refs/main').read_text()
    graded = out / 'graded'
    evaluator.prepare(graded, 'AllMiniLML6V2', revision)
    evaluator.collect(graded, args.codanna, cache, True)
    report = evaluator.evaluate(graded)
    evaluator.write_json(graded / 'report.json', report)
    summary['runs']['graded'] = {'status': report['status'], 'summary': report['summary']}
    evaluator.write_json(out / 'summary.json', summary)
    print(out / 'summary.json')
    return int(any(row.get('exit_code', 0) != 0 for row in summary['runs'].values())
               or report['status'] != 'measured')


if __name__ == '__main__':
    sys.exit(main())

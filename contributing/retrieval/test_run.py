"""Evaluator contract tests; these do not claim that Codanna retrieval passes."""
from __future__ import annotations

import copy
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import runpy
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('retrieval_runner', HERE / 'run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class EvaluatorTests(unittest.TestCase):
    def test_body_policy_is_explicit_and_default_remains_comment_based(self):
        for policy in ('doc_comment', 'symbol_body_v1'):
            with tempfile.TemporaryDirectory() as temp:
                config = runner.write_settings(Path(temp), True, policy)
                settings = tomllib.loads(config.read_text())
                self.assertEqual(settings['semantic_search']['code_representation'], policy)
        with tempfile.TemporaryDirectory() as temp:
            config = runner.write_settings(Path(temp), True)
            self.assertEqual(tomllib.loads(config.read_text())['semantic_search']['code_representation'], 'doc_comment')

    def setUp(self):
        self.manifest = json.loads((HERE / 'cases.json').read_text(encoding='utf-8'))
        self.workspace = HERE / 'workspace'

    def case(self, key):
        return copy.deepcopy(next(c for c in self.manifest['cases'] if c['id'] == key))

    def test_manifest_and_fixture_syntax(self):
        runner.validate_manifest(self.manifest)
        self.assertEqual(len(self.manifest['cases']), 32)
        self.assertEqual(len(self.manifest['manual_cases']), 10)

    def test_duplicate_case_id_is_rejected(self):
        self.manifest['cases'].append(self.case('D01'))
        with self.assertRaises(ValueError):
            runner.validate_manifest(self.manifest)

    def test_missing_gold_fact_is_rejected(self):
        self.manifest['cases'] = [self.case('D01')]
        self.manifest['cases'][0]['required'][0]['contains'] = 'NONEXISTENT_GOLD_504'
        with self.assertRaises(ValueError):
            runner.validate_manifest(self.manifest)

    def test_existing_file_outside_workspace_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.source_path(self.workspace, '../cases.json')

    def test_error_envelopes_never_become_not_found(self):
        for status, code, rc in [('error', 'INDEX_ERROR', 2), ('partial_success', 'OK', 0),
                                 ('success', 'OK', 2), ('not_found', 'INDEX_ERROR', 1)]:
            with self.subTest(status=status, code=code, rc=rc), self.assertRaises(ValueError):
                runner.envelope_items({'status': status, 'code': code, 'data': []}, rc)

    def test_not_found_contract_and_nested_items(self):
        self.assertEqual(runner.envelope_items({'status': 'not_found', 'code': 'NOT_FOUND', 'data': None}, 1), [])
        self.assertEqual(runner.envelope_items({'status': 'success', 'code': 'OK', 'data': {'items': [{}]}}, 0), [{}])
        with self.assertRaises(ValueError):
            runner.envelope_items({'status': 'success', 'code': 'OK', 'data': {'text': 'no results'}}, 0)

    def test_wrong_chunk_of_correct_document_does_not_pass(self):
        path = self.workspace / 'docs/long-runbook.md'
        item = {'source_path': str(path), 'byte_range': [0, 100]}
        rows = runner.normalize_rows([item], 'documents', self.workspace)
        result = runner.score_retrieval(self.case('D03'), rows)
        self.assertFalse(result['passed'])
        self.assertEqual(result['required_evidence_recall_at_5'], 0)

    def test_exact_chunk_fact_passes(self):
        path = self.workspace / 'docs/long-runbook.md'
        start = path.read_bytes().index(b'violet-heron')
        rows = runner.normalize_rows([{'source_path': str(path), 'byte_range': [start, start + 12]}],
                                     'documents', self.workspace)
        self.assertTrue(runner.score_retrieval(self.case('D03'), rows)['passed'])

    def test_out_of_bounds_and_split_unicode_are_rejected(self):
        path = self.workspace / 'docs/unicode.md'
        position = next(i for i, byte in enumerate(path.read_bytes()) if byte >= 128)
        for bounds in ([0, len(path.read_bytes()) + 1], [position, position + 1]):
            with self.subTest(bounds=bounds), self.assertRaises(ValueError):
                runner.normalize_rows([{'source_path': str(path), 'byte_range': bounds}], 'documents', self.workspace)

    def test_ignored_result_is_rejected(self):
        path = self.workspace / 'docs/private/secret.md'
        with self.assertRaises(ValueError):
            runner.normalize_rows([{'source_path': str(path), 'byte_range': [0, 10]}], 'documents', self.workspace)

    def test_rank_cutoff_and_maximum_rank(self):
        rows = [{'path': 'docs/retention.md', 'name': None, 'text': ''}] * 3
        rows.append({'path': 'docs/ADR-004.md', 'name': None, 'text': '8,388,609'})
        result = runner.score_retrieval(self.case('D01'), rows)
        self.assertFalse(result['passed'])
        self.assertEqual(result['ranks'], [4])
        self.assertEqual(result['hit_at_5'], 1)
        self.assertEqual(result['reciprocal_rank_at_5'], 0.25)
        rows.insert(0, rows[0])
        rows.insert(0, rows[0])
        self.assertEqual(runner.score_retrieval(self.case('D01'), rows)['hit_at_5'], 0)

    def test_duplicates_do_not_manufacture_diversity(self):
        rows = [{'path': 'docs/ADR-004.md', 'name': None, 'text': ''}] * 5
        self.assertFalse(runner.score_retrieval(self.case('D06'), rows)['passed'])

    def test_domain_distractor_is_forbidden_as_first_result(self):
        rows = [{'path': 'src/accounts.py', 'name': 'validate', 'text': ''},
                {'path': 'src/images.py', 'name': 'validate', 'text': ''}]
        result = runner.score_retrieval(self.case('S03'), rows)
        self.assertFalse(result['passed'])
        self.assertEqual(result['forbidden_evidence'], 1)

    def test_empty_oracle_rejects_arbitrary_documents(self):
        self.assertTrue(runner.score_retrieval(self.case('D04'), [])['passed'])
        self.assertFalse(runner.score_retrieval(self.case('D04'), [{'path': 'docs/ADR-004.md'}])['passed'])

    def test_duplicate_name_is_not_duplicate_identity(self):
        node = {'kind': 'symbol', 'label': 'images::validate', 'source': {'path': 'src/images.py'}}
        self.assertTrue(runner.matches(node, {'path': 'src/images.py', 'name': 'validate'}))
        self.assertFalse(runner.matches(node, {'path': 'src/accounts.py', 'name': 'validate'}))

    def test_duplicate_heading_requires_matching_range(self):
        node = {'kind': 'section', 'label': 'Retry', 'source': {'path': 'docs/navigation.md', 'start_line': 3}}
        self.assertFalse(runner.matches(node, {'path': 'docs/navigation.md', 'label': 'Retry', 'start_line': 7}))

    def test_stale_hash_bad_lines_and_cross_scope_are_rejected(self):
        good = {'repo': 'primary', 'path': 'src/storage.py', 'start_line': 4, 'end_line': 7,
                'hash': runner.digest(self.workspace / 'src/storage.py')}
        runner.verify_span(good, self.workspace, 'primary')
        for change in ({'hash': '0' * 64}, {'start_line': 0}, {'end_line': 9999}, {'repo': 'peer'}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                runner.verify_span(good | change, self.workspace, 'primary')

    def test_dangling_graph_edge_is_rejected(self):
        graph = {'schema_version': 1, 'repositories': {'primary': {}}, 'nodes': {},
                 'edges': [{'from': 'missing', 'to': 'also-missing'}], 'unresolved': []}
        with self.assertRaises(ValueError):
            runner.verify_graph(graph, self.workspace, 'primary')

    def test_unmeasured_semantics_are_not_fabricated_metrics(self):
        summary = runner.summarize([{'tier': 'invariant', 'passed': True}], self.manifest['minimums'])
        self.assertIsNone(summary['metrics']['hit_at_5'])
        self.assertIsNone(summary['metrics']['mrr_at_5'])
        self.assertFalse(runner.summarize([], self.manifest['minimums'])['selected_cases_passed'])

    def test_failed_retrieval_remains_in_metric_denominator(self):
        results = [{'tier': 'target', 'passed': True, 'hit_at_5': 1, 'reciprocal_rank_at_5': 1,
                    'required_evidence_recall_at_5': 1},
                   {'tier': 'target', 'passed': False, 'hit_at_5': 0, 'reciprocal_rank_at_5': 0,
                    'required_evidence_recall_at_5': 0}]
        summary = runner.summarize(results, self.manifest['minimums'])
        self.assertEqual(summary['metrics']['hit_at_5'], 0.5)
        self.assertFalse(summary['measured_minimums_met'])
        self.assertFalse(summary['selected_cases_passed'])

    def test_provider_credentials_and_overrides_are_not_inherited(self):
        with patch.dict(os.environ, {'OPENAI_API_KEY': 'synthetic', 'CODANNA_EMBED_URL': 'invalid',
                                     'CI_SEMANTIC_SEARCH__ENABLED': 'true'}, clear=True):
            env = runner.isolated_env(Path('/synthetic-home'))
        self.assertFalse(set(env) & {'OPENAI_API_KEY', 'CODANNA_EMBED_URL', 'CI_SEMANTIC_SEARCH__ENABLED'})

    def test_generated_config_is_local_and_explicit(self):
        for enabled in (False, True):
            with tempfile.TemporaryDirectory() as temp:
                config = runner.write_settings(Path(temp), enabled)
                data = tomllib.loads(config.read_text(encoding='utf-8'))
                self.assertEqual(data['semantic_search']['enabled'], enabled)
                self.assertFalse(any(k.startswith('remote') for k in data['semantic_search']))
                self.assertEqual(data['documents']['collections']['docs']['paths'], ['docs'])

    def test_timeout_and_exit_errors_are_explicit(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = runner.Commands(root, runner.isolated_env(root))
            with self.assertRaises(RuntimeError):
                commands.call([sys.executable, '-c', 'import time; time.sleep(10)'], root, timeout=0.1)
            self.assertEqual(commands.history[0]['error'], 'timeout')
            with self.assertRaises(RuntimeError):
                commands.call([sys.executable, '-c', 'raise SystemExit(3)'], root)
            self.assertEqual(commands.history[1]['returncode'], 3)

    def test_setup_failure_writes_an_explicit_failure_report(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / 'result'
            # Python answers --version but rejects Codanna's --config command.
            argv = ['run.py', '--codanna', sys.executable, '--knowledge', sys.executable,
                    '--out', str(out)]
            with patch.object(sys, 'argv', argv), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(runner.main(), 2)
            report = json.loads((out / 'report.json').read_text(encoding='utf-8'))
            self.assertEqual(report['status'], 'setup_error')
            self.assertFalse(report['full_qualification'])
            self.assertTrue(report['commands'])

    def test_fixture_behavior_is_self_consistent(self):
        # This validates only the miniature application's behavior, not its index.
        with patch.object(sys, 'path', [str(self.workspace / 'src')] + sys.path):
            tests = runpy.run_path(str(self.workspace / 'tests/test_delivery.py'))
            for name, value in tests.items():
                if name.startswith('test_'):
                    value()


if __name__ == '__main__':
    unittest.main()

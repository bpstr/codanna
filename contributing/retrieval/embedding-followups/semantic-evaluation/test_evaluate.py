"""Deterministic evaluator contracts. Never load a model or launch Codanna."""
from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('semantic_evaluation', HERE / 'evaluate.py')
evaluation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluation)


def metric_query(grades):
    return {'judgments': [{'path': path, 'grade': grade} for path, grade in grades.items()]}


class MetricTests(unittest.TestCase):
    def test_graded_ndcg_uses_exponential_gain_and_original_ranks(self):
        query = metric_query({'a': 3, 'b': 2, 'c': 1, 'd': 0})
        result = evaluation.score_query(query, ['b', 'a', 'd', 'c'])
        actual = 3 + 7 / math.log2(3) + 1 / math.log2(5)
        ideal = 7 + 3 / math.log2(3) + 1 / math.log2(4)
        self.assertAlmostEqual(result['source_ndcg_at_5'], actual / ideal)
        self.assertEqual(result['source_recall_at_5'], 1)

    def test_duplicate_sources_cannot_inflate_gain_or_recall(self):
        query = metric_query({'a': 3, 'b': 3, 'c': 0})
        result = evaluation.score_query(query, ['a', 'a', 'a', 'a', 'a', 'b'])
        self.assertAlmostEqual(result['source_ndcg_at_5'], 7 / (7 + 7 / math.log2(3)))
        self.assertEqual(result['source_recall_at_5'], 0.5)
        self.assertEqual(result['grade_3_source_recall_at_5'], 0.5)
        self.assertEqual(result['duplicate_slots_at_5'], 4)
        self.assertEqual(result['duplicate_fraction_at_5'], 0.8)

    def test_duplicate_does_not_compress_later_relevant_rank(self):
        result = evaluation.score_query(metric_query({'a': 0, 'b': 3}), ['a', 'a', 'b'])
        self.assertEqual(result['source_reciprocal_rank_at_5'], 1 / 3)
        self.assertAlmostEqual(result['source_ndcg_at_5'], 0.5)

    def test_grade_one_has_graded_gain_but_is_not_binary_relevance(self):
        result = evaluation.score_query(metric_query({'a': 1, 'b': 3}), ['a'])
        self.assertGreater(result['source_ndcg_at_5'], 0)
        self.assertEqual(result['source_hit_at_5'], 0)
        self.assertEqual(result['source_recall_at_5'], 0)

    def test_missing_results_score_zero_for_positive_query(self):
        result = evaluation.score_query(metric_query({'a': 3}), [])
        self.assertEqual(result['source_ndcg_at_5'], 0)
        self.assertEqual(result['source_reciprocal_rank_at_5'], 0)
        self.assertEqual(result['source_recall_at_5'], 0)

    def test_all_zero_judgments_are_not_positive_denominators(self):
        result = evaluation.score_query(metric_query({'a': 0}), ['a'])
        self.assertIsNone(result['source_ndcg_at_5'])
        self.assertIsNone(result['source_recall_at_5'])
        self.assertEqual(result['grade_zero_sources_at_5'], 1)
        self.assertTrue(result['all_zero_judgments'])

    def test_unjudged_source_is_an_error(self):
        with self.assertRaises(ValueError):
            evaluation.score_query(metric_query({'a': 3}), ['foreign'])

    def test_failed_queries_remain_in_aggregate_denominators(self):
        query = metric_query({'a': 3})
        rows = [{'metrics': evaluation.score_query(query, ['a'])},
                {'metrics': evaluation.score_query(query, []), 'error': 'fixture failure'}]
        result = evaluation.aggregate(rows)
        self.assertEqual(result['means']['source_recall_at_5'], 0.5)
        self.assertEqual(result['denominators']['source_recall_at_5'], 2)
        self.assertEqual(result['error_count'], 1)


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name) / 'run'
        self.manifest = evaluation.read_json(evaluation.MANIFEST)
        self.experiment = evaluation.prepare(self.out, 'AllMiniLML6V2', 'synthetic-revision')
        self.query = self.manifest['queries'][0]
        self.source = next(row['path'] for row in self.query['judgments'] if row['grade'] == 3)
        self.binary = self.out.parent / 'fixture-codanna'
        self.binary.write_text('This is a fixture and must never be executed.\n', encoding='utf-8')
        self.binary.chmod(0o700)
        self.base = [str(self.binary), '--config', str(self.out / 'settings.toml'), 'documents']

    def response(self, paths=None):
        paths = [self.source] if paths is None else paths
        rows = [{'chunk_id': index, 'collection': evaluation.COLLECTION,
                 'source_path': str(self.out / 'workspace' / path),
                 'byte_range': [0, (self.out / 'workspace' / path).stat().st_size],
                 'similarity': 0.5, 'heading_context': [], 'content_preview': 'fixture'}
                for index, path in enumerate(paths, 1)]
        return {'type': 'result', 'status': 'success', 'code': 'OK', 'exit_code': 0,
                'message': 'synthetic contract fixture', 'data': rows,
                'meta': {'schema_version': '1.0.0', 'entity_type': 'document',
                         'query': self.query['text'], 'count': len(rows)}}

    def normalize(self, envelope, code=0):
        return evaluation.normalize_response(envelope, code, self.query, self.out / 'workspace',
                                             self.experiment['sources'])

    def stats(self):
        identity = {'backend': 'local', 'model': self.experiment['model'], 'endpoint_sha256': None,
                    'model_revision': self.experiment['model_revision'],
                    'input_policy': 'complete-input-v2:huggingface-tokenizers-0.22.2:' + 'a' * 64 + ':256'}
        return {'name': evaluation.COLLECTION, 'file_count': len(self.experiment['sources']),
                'chunk_count': 28,
                'embedding_index': {'generation': 'synthetic-generation',
                                    'identity': json.dumps(identity) + ';document-input=2',
                                    'unembedded_chunks': 0, 'live_vectors': 28, 'physical_vectors': 28}}

    def record(self, name, value, argv, code=0):
        raw = self.out / 'raw'
        raw.mkdir(exist_ok=True)
        stdout, stderr = raw / (name + '.json'), raw / (name + '.stderr')
        if isinstance(value, str):
            stdout.write_text(value, encoding='utf-8')
        else:
            evaluation.write_json(stdout, value)
        stderr.write_text('', encoding='utf-8')
        return {'argv': argv, 'returncode': code, 'stdout': stdout.relative_to(self.out).as_posix(),
                'stderr': stderr.relative_to(self.out).as_posix(),
                'stdout_sha256': evaluation.sha256(stdout), 'stderr_sha256': evaluation.sha256(stderr)}

    def fixture_cache(self, cache):
        cache.mkdir(parents=True)
        repo, weight = evaluation.LOCAL_MODELS[self.experiment['model']]
        ref = cache / repo / 'refs/main'
        ref.parent.mkdir(parents=True)
        ref.write_text('b' * 40, encoding='utf-8')
        snapshot = cache / repo / 'snapshots' / ('b' * 40)
        snapshot.mkdir(parents=True)
        self.weight_path = snapshot / weight
        self.weight_path.parent.mkdir(parents=True, exist_ok=True)
        self.weight_path.write_bytes(b'fixture bytes, not an ONNX model')
        for name in ('tokenizer.json', 'config.json', 'special_tokens_map.json', 'tokenizer_config.json'):
            (snapshot / name).write_text('{}', encoding='utf-8')
        return evaluation.cache_inventory(cache, self.experiment['model'])

    def fixture_capture(self):
        files = self.fixture_cache(self.out / 'home/.codanna/models')
        capture = {'schema_version': 1, 'origin': 'manual-local-cli', 'status': 'collected',
                   'experiment_sha256': evaluation.sha256(self.out / 'experiment.json'),
                   'binary': {'path': str(self.binary), 'sha256': evaluation.sha256(self.binary)},
                   'cached_artifacts': files, 'cached_artifacts_sha256': evaluation.inventory_hash(files),
                   'version': self.record('version', 'codanna fixture version\n', [str(self.binary), '--version']),
                   'index': self.record('index', 'fixture indexing output\n', self.base + ['index', '--collection', evaluation.COLLECTION, '--no-progress']),
                   'stats_before': self.record('stats-before', self.stats(), self.base + ['stats', evaluation.COLLECTION, '--json']),
                   'stats_after': self.record('stats-after', self.stats(), self.base + ['stats', evaluation.COLLECTION, '--json']),
                   'responses': []}
        for query in self.manifest['queries']:
            self.query = query
            paths = [row['path'] for row in query['judgments'] if row['grade'] >= 2]
            record = self.record(query['id'], self.response(paths), self.base + ['search', query['text'],
                                 '--collection', evaluation.COLLECTION, '--limit', '5', '--json'])
            capture['responses'].append({'query_id': query['id'], 'query_sha256': evaluation.text_hash(query['text']),
                                         'command': record})
        evaluation.write_json(self.out / 'capture.json', capture)
        return capture

    def test_frozen_dataset_and_unrun_status(self):
        evaluation.validate_manifest(self.manifest, evaluation.CORPUS)
        self.assertFalse(self.experiment['full_qualification'])
        self.assertFalse(self.experiment['semantic_relevance_measured'])
        self.assertEqual(self.experiment['status'], 'unrun')

    def test_preparation_keeps_labels_and_outputs_outside_indexed_docs(self):
        config = tomllib.loads((self.out / 'settings.toml').read_text(encoding='utf-8'))
        self.assertEqual(config['documents']['collections'][evaluation.COLLECTION]['paths'], ['docs'])
        self.assertEqual(evaluation.inventory(self.out / 'workspace'), self.experiment['sources'])
        self.assertNotIn('queries.json', self.experiment['sources'])
        self.assertFalse(any(key.startswith('remote') for key in config['semantic_search']))

    def test_full_query_leakage_and_unknown_corpus_files_are_rejected(self):
        target = self.out / 'workspace' / self.source
        target.write_text(target.read_text(encoding='utf-8') + '\n' + self.query['text'], encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'leaked'):
            evaluation.validate_manifest(self.manifest, self.out / 'workspace')
        (self.out / 'workspace' / 'answers.json').write_text('{}', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'Only the fixed'):
            evaluation.validate_manifest(self.manifest, self.out / 'workspace')

    def test_missing_duplicate_and_invalid_judgments_are_rejected(self):
        for change in ('missing', 'duplicate', 'grade', 'rationale'):
            manifest = copy.deepcopy(self.manifest)
            rows = manifest['queries'][0]['judgments']
            if change == 'missing':
                rows.pop()
            elif change == 'duplicate':
                rows.append(rows[0])
            elif change == 'grade':
                rows[0]['grade'] = True
            else:
                rows[0]['rationale'] = ''
            with self.subTest(change=change), self.assertRaises(ValueError):
                evaluation.validate_manifest(manifest, evaluation.CORPUS)

    def test_envelope_ties_preserve_cli_order(self):
        other = next(path for path in self.experiment['sources'] if path != self.source)
        rows = self.normalize(self.response([other, self.source]))
        self.assertEqual([row['path'] for row in rows], [other, self.source])

    def test_empty_result_is_distinct_from_protocol_failure(self):
        envelope = self.response([])
        envelope.update(status='not_found', code='NOT_FOUND', exit_code=1, data=None)
        self.assertEqual(self.normalize(envelope, 1), [])
        envelope.update(status='error', code='INDEX_ERROR')
        with self.assertRaises(ValueError):
            self.normalize(envelope, 2)

    def test_malformed_envelopes_and_nonfinite_cosines_are_rejected(self):
        for value in (None, [], {'meta': None}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.normalize(value)
        for score in (float('nan'), float('inf'), -float('inf'), True, '0.5', 2):
            envelope = self.response()
            envelope['data'][0]['similarity'] = score
            with self.subTest(score=score), self.assertRaises(ValueError):
                self.normalize(envelope)

    def test_stale_hash_foreign_path_and_symlink_are_rejected(self):
        envelope = self.response()
        original = copy.deepcopy(envelope)
        envelope['data'][0]['source_path'] = str(self.binary)
        with self.assertRaises(ValueError):
            self.normalize(envelope)
        link = self.out / 'workspace/docs/outside.md'
        link.symlink_to(self.binary)
        envelope['data'][0]['source_path'] = str(link)
        with self.assertRaises(ValueError):
            self.normalize(envelope)
        (self.out / 'workspace' / self.source).write_text('changed', encoding='utf-8')
        with self.assertRaises(ValueError):
            self.normalize(original)

    def test_workspace_parent_alias_preserves_results_without_allowing_source_symlinks(self):
        alias = self.out.parent / 'alias'
        alias.symlink_to(self.out.resolve(), target_is_directory=True)
        envelope = self.response()
        envelope['data'][0]['source_path'] = str(alias / 'workspace' / self.source)
        self.assertEqual(self.normalize(envelope)[0]['path'], self.source)
        link = self.out / 'workspace/docs/linked.md'
        link.symlink_to((self.out / 'workspace' / self.source).resolve())
        envelope['data'][0]['source_path'] = str(alias / 'workspace/docs/linked.md')
        with self.assertRaises(ValueError):
            self.normalize(envelope)

    def test_bad_utf8_span_duplicate_chunks_and_foreign_collections_are_rejected(self):
        for change in ('range', 'duplicate', 'collection', 'query', 'budget'):
            envelope = self.response()
            if change == 'range':
                envelope['data'][0]['byte_range'] = [0, 999999]
            elif change == 'duplicate':
                envelope['data'] *= 2
                envelope['meta']['count'] = 2
            elif change == 'collection':
                envelope['data'][0]['collection'] = 'foreign'
            elif change == 'query':
                envelope['meta']['query'] = 'another question'
            else:
                envelope['data'] *= 6
                envelope['meta']['count'] = 6
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.normalize(envelope)
        path = self.out / 'workspace' / self.source
        with path.open('ab') as stream:
            stream.write('é'.encode('utf-8'))
        self.experiment['sources'][self.source] = evaluation.sha256(path)
        envelope = self.response()
        envelope['data'][0]['byte_range'] = [path.stat().st_size - 1, path.stat().st_size]
        with self.assertRaises(UnicodeDecodeError):
            self.normalize(envelope)

    def test_diagnostics_require_complete_vectors_and_actual_local_identity(self):
        for field, value in [('unembedded_chunks', 1), ('live_vectors', 0),
                             ('identity', None), ('generation', None)]:
            stats = self.stats()
            stats['embedding_index'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                evaluation.validate_diagnostics(stats, self.experiment)
        stats = self.stats()
        stats['embedding_index']['identity'] = stats['embedding_index']['identity'].replace('"local"', '"remote"')
        with self.assertRaises(ValueError):
            evaluation.validate_diagnostics(stats, self.experiment)
        for value in (None, []):
            with self.assertRaises(ValueError):
                evaluation.validate_diagnostics(value, self.experiment)

    def test_environment_scrubs_credentials_and_targets_private_cache(self):
        with patch.dict(os.environ, {'OPENAI_API_KEY': 'synthetic', 'HF_TOKEN': 'synthetic',
                                    'CODANNA_EMBED_URL': 'invalid', 'CI_SEMANTIC_SEARCH__ENABLED': 'false',
                                    'HF_ENDPOINT': 'https://unexpected.invalid', 'HTTPS_PROXY': 'unexpected'}, clear=True):
            env = evaluation.isolated_env(self.out / 'home')
        self.assertFalse(set(env) & {'OPENAI_API_KEY', 'HF_TOKEN', 'CODANNA_EMBED_URL', 'CI_SEMANTIC_SEARCH__ENABLED', 'HTTPS_PROXY'})
        self.assertEqual(env['HF_HOME'], str(self.out / 'home/.codanna/models'))
        self.assertEqual(env['HF_ENDPOINT'], 'codanna-offline://cache')

    def test_collection_requires_explicit_opt_in_before_any_process(self):
        with patch.object(evaluation.subprocess, 'Popen', side_effect=AssertionError('process forbidden')):
            with self.assertRaisesRegex(ValueError, 'allow-local-model'):
                evaluation.collect(self.out, self.binary, self.out, False)

    def test_live_stdout_and_stderr_limits_stop_local_fixture_processes(self):
        for stream, descriptor in (('stdout', 1), ('stderr', 2)):
            capture_root = self.out.parent / f'bounded-{stream}'
            capture_root.mkdir()
            commands = evaluation.Commands(capture_root, evaluation.isolated_env(capture_root))
            program = f'import os,time; os.write({descriptor}, b"x" * 1048576); time.sleep(30)'
            with self.subTest(stream=stream), patch.object(evaluation, 'MAX_JSON_BYTES', 4096):
                with self.assertRaisesRegex(ValueError, f'{stream}_output_limit'):
                    commands.call([sys.executable, '-c', program], self.out, timeout=5)
                entry = commands.history[0]
                self.assertEqual(entry['error'], f'{stream}_output_limit')
                self.assertNotEqual(entry['returncode'], 0)
                self.assertEqual((capture_root / entry[stream]).stat().st_size, 4096)
                self.assertLess(entry['elapsed_seconds'], 5)

    def test_timeout_stops_local_fixture_process_and_retains_bounded_output(self):
        commands = evaluation.Commands(self.out, evaluation.isolated_env(self.out))
        with self.assertRaisesRegex(ValueError, 'timeout'):
            commands.call([sys.executable, '-c', 'import time; time.sleep(30)'], self.out, timeout=0.1)
        self.assertEqual(commands.history[0]['error'], 'timeout')
        self.assertNotEqual(commands.history[0]['returncode'], 0)

    def test_imported_stdout_and_stderr_are_bounded_before_hashing(self):
        for stream in ('stdout', 'stderr'):
            entry = self.record(f'large-{stream}', 'ok', [])
            path = self.out / entry[stream]
            path.write_bytes(b'x' * 129)
            entry[stream + '_sha256'] = evaluation.sha256(path)
            with self.subTest(stream=stream), patch.object(evaluation, 'MAX_JSON_BYTES', 128):
                with self.assertRaisesRegex(ValueError, f'Captured {stream} exceeds'):
                    evaluation.captured_bytes(self.out, entry)

    def test_cache_preflight_rejects_missing_assets_and_nonexact_snapshot_ref(self):
        cache = self.out.parent / 'cache'
        self.fixture_cache(cache)
        repo, _ = evaluation.LOCAL_MODELS[self.experiment['model']]
        ref = cache / repo / 'refs/main'
        with patch.object(evaluation.subprocess, 'Popen', side_effect=AssertionError('process forbidden')):
            ref.write_text('b' * 40 + '\n', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'without a newline'):
                evaluation.collect(self.out, self.binary, cache, True)
            ref.write_text('b' * 40, encoding='utf-8')
            self.weight_path.with_name('config.json').unlink()
            with self.assertRaisesRegex(ValueError, 'Missing or escaping'):
                evaluation.collect(self.out, self.binary, cache, True)

    def test_both_supported_models_require_exact_cached_assets(self):
        for model in evaluation.LOCAL_MODELS:
            self.experiment['model'] = model
            cache = self.out.parent / model
            files = self.fixture_cache(cache)
            with self.subTest(model=model):
                self.assertEqual(len(files), 6)
                self.assertTrue(any(path.endswith(evaluation.LOCAL_MODELS[model][1]) for path in files))

    def test_manual_collection_uses_mocked_transport_and_only_selected_snapshot(self):
        cache = self.out.parent / 'cache'
        files = self.fixture_cache(cache)
        (cache / 'unrelated-model.onnx').write_bytes(b'unrelated fixture must not be copied or hashed')
        owner = self

        class FakeCommands:
            def __init__(self, out, env):
                self.history = []
                owner.assertEqual(env['HF_ENDPOINT'], 'codanna-offline://cache')

            def call(self, argv, cwd, timeout=120):
                if argv[-1] == '--version':
                    value = 'codanna mocked CLI\n'
                elif 'index' in argv:
                    value = 'mocked indexing output\n'
                elif 'stats' in argv:
                    value = owner.stats()
                else:
                    text = argv[argv.index('search') + 1]
                    owner.query = next(query for query in owner.manifest['queries'] if query['text'] == text)
                    paths = [row['path'] for row in owner.query['judgments'] if row['grade'] >= 2]
                    value = owner.response(paths)
                entry = owner.record(f'mocked-{len(self.history)}', value, argv)
                self.history.append(entry)
                return entry

        with patch.object(evaluation, 'Commands', FakeCommands), patch.object(
                evaluation.subprocess, 'Popen', side_effect=AssertionError('process forbidden')):
            capture = evaluation.collect(self.out, self.binary, cache, True)
            report = evaluation.evaluate(self.out)
        self.assertEqual(capture['status'], 'collected')
        self.assertEqual(capture['cached_artifacts'], files)
        self.assertEqual(evaluation.inventory(self.out / 'home/.codanna/models'), files)
        self.assertEqual(len(capture['commands']), 14)
        self.assertEqual(report['status'], 'measured')
        self.assertFalse(report['full_qualification'])

    def test_score_uses_fixture_json_only_and_never_claims_full_qualification(self):
        self.fixture_capture()
        with patch.object(evaluation.subprocess, 'Popen', side_effect=AssertionError('process forbidden')):
            report = evaluation.evaluate(self.out)
        self.assertEqual(report['status'], 'measured')
        self.assertFalse(report['full_qualification'])
        self.assertEqual(report['summary']['query_count'], 10)
        self.assertEqual(report['raw_cosine_comparison']['status'], 'unsupported_by_public_cli')

    def test_missing_query_is_an_error_and_stays_in_denominator(self):
        capture = self.fixture_capture()
        capture['responses'].pop()
        evaluation.write_json(self.out / 'capture.json', capture)
        report = evaluation.evaluate(self.out)
        self.assertEqual(report['status'], 'incomplete')
        self.assertFalse(report['semantic_relevance_measured'])
        self.assertEqual(report['summary']['denominators']['source_recall_at_5'], 10)
        self.assertEqual(report['summary']['means']['source_recall_at_5'], 0.9)

    def test_changed_raw_response_is_incomplete_but_changed_generation_is_invalid(self):
        capture = self.fixture_capture()
        (self.out / capture['responses'][0]['command']['stdout']).write_text('{}', encoding='utf-8')
        report = evaluation.evaluate(self.out)
        self.assertEqual(report['status'], 'incomplete')
        after = self.stats()
        after['embedding_index']['generation'] = 'different-generation'
        capture['stats_after'] = self.record('changed-stats', after, self.base + ['stats', evaluation.COLLECTION, '--json'])
        evaluation.write_json(self.out / 'capture.json', capture)
        with self.assertRaisesRegex(ValueError, 'generation or counts changed'):
            evaluation.evaluate(self.out)

    def test_stable_generation_with_changed_vector_counts_invalidates_capture(self):
        capture = self.fixture_capture()
        after = self.stats()
        after['chunk_count'] = 29
        after['embedding_index'].update(live_vectors=29, physical_vectors=29)
        capture['stats_after'] = self.record('changed-counts', after, self.base + ['stats', evaluation.COLLECTION, '--json'])
        evaluation.write_json(self.out / 'capture.json', capture)
        with self.assertRaisesRegex(ValueError, 'counts changed'):
            evaluation.evaluate(self.out)

    def test_invalid_global_model_identity_writes_no_aggregate(self):
        capture = self.fixture_capture()
        stats = self.stats()
        stats['embedding_index']['identity'] = stats['embedding_index']['identity'].replace('"local"', '"remote"')
        for phase in ('before', 'after'):
            capture['stats_' + phase] = self.record('invalid-' + phase, stats, self.base + ['stats', evaluation.COLLECTION, '--json'])
        evaluation.write_json(self.out / 'capture.json', capture)
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(evaluation.main(['score', '--run', str(self.out)]), 2)
        report = evaluation.read_json(self.out / 'report.json')
        self.assertEqual(report['status'], 'invalid_capture')
        self.assertIsNone(report['summary'])
        self.assertFalse(report['semantic_relevance_measured'])

    def test_query_failure_without_final_stats_keeps_incomplete_denominators(self):
        capture = self.fixture_capture()
        capture.pop('stats_after')
        capture['responses'].pop()
        capture.update(status='collection_error', error='synthetic query failure')
        evaluation.write_json(self.out / 'capture.json', capture)
        report = evaluation.evaluate(self.out)
        self.assertEqual(report['status'], 'incomplete')
        self.assertEqual(report['summary']['denominators']['source_recall_at_5'], 10)
        self.assertEqual(report['summary']['means']['source_recall_at_5'], 0.9)

    def test_missing_version_or_changed_command_is_incomplete(self):
        capture = self.fixture_capture()
        capture['version']['argv'] = ['wrong-binary', '--version']
        evaluation.write_json(self.out / 'capture.json', capture)
        self.assertEqual(evaluation.evaluate(self.out)['status'], 'incomplete')

    def test_stale_manifest_or_model_bytes_is_invalid(self):
        self.fixture_capture()
        self.weight_path.write_bytes(b'changed model fixture')
        with self.assertRaisesRegex(ValueError, 'Stale cached'):
            evaluation.evaluate(self.out)
        (self.out / 'queries.json').write_text('{}', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'Stale query'):
            evaluation.evaluate(self.out)

    def test_invalid_score_writes_explicit_unqualified_report(self):
        (self.out / 'capture.json').write_text('[]', encoding='utf-8')
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(evaluation.main(['score', '--run', str(self.out)]), 2)
        report = evaluation.read_json(self.out / 'report.json')
        self.assertEqual(report['status'], 'invalid_capture')
        self.assertFalse(report['full_qualification'])
        self.assertIsNone(report['summary'])

    def test_strict_json_rejects_nonfinite_and_duplicate_keys(self):
        for text in ('{"score": NaN}', '{"score": Infinity}', '{"score": 1e309}', '{"a": 1, "a": 2}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                evaluation.strict_json(text)


if __name__ == '__main__':
    unittest.main()

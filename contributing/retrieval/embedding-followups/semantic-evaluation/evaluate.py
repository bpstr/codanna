#!/usr/bin/env python3
"""Graded document relevance: model-free checks/scoring, explicitly manual collection.

Python 3.11+, standard library only. No model is used by check, prepare, or score.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
CORPUS = HERE / 'corpus'
MANIFEST = HERE / 'queries.json'
COLLECTION = 'semantic_eval'
K = 5
MAX_JSON_BYTES = 16 * 1024 * 1024
SHA256 = re.compile(r'[0-9a-f]{64}')
LOCAL_MODELS = {
    'AllMiniLML6V2': ('models--Qdrant--all-MiniLM-L6-v2-onnx', 'model.onnx'),
    'MultilingualE5Small': ('models--intfloat--multilingual-e5-small', 'onnx/model.onnx'),
}
RAW_COMPARISON = {
    'status': 'unsupported_by_public_cli',
    'reason': 'documents search exposes selected results only; sorting their cosine scores '
              'cannot recover omitted candidates or measure a raw-cosine baseline.',
    'raw_cosine_metrics': None,
}


def fail(message: str) -> None:
    raise ValueError(message)


def nonempty(value, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f'{name} must be a nonempty string')
    return value


def object_value(value, name: str) -> dict:
    if not isinstance(value, dict):
        fail(f'{name} must be an object')
    return value


def integer(value, name: str, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        fail(f'{name} must be an integer >= {minimum}')
    return value


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def text_hash(text: str) -> str:
    return hashlib.sha256(text.encode('utf-8')).hexdigest()


def strict_json(text: str):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                fail(f'Duplicate JSON key: {key}')
            result[key] = value
        return result
    def finite_float(value):
        parsed = float(value)
        if not math.isfinite(parsed):
            fail(f'Nonfinite JSON number: {value}')
        return parsed
    return json.loads(text, object_pairs_hook=pairs, parse_float=finite_float,
                      parse_constant=lambda value: fail(f'Nonfinite JSON number: {value}'))


def read_json(path: Path):
    with path.open('rb') as stream:
        raw = stream.read(MAX_JSON_BYTES + 1)
    if len(raw) > MAX_JSON_BYTES:
        fail(f'JSON exceeds {MAX_JSON_BYTES} bytes: {path}')
    return strict_json(raw.decode('utf-8'))


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + '\n',
                    encoding='utf-8')


def contained_file(root: Path, value: str) -> Path:
    nonempty(value, 'path')
    relative = Path(value)
    if relative.is_absolute() or '..' in relative.parts:
        fail(f'Expected contained relative path: {value}')
    root = root.resolve()
    path = root / relative
    if not path.resolve().is_relative_to(root) or not path.is_file():
        fail(f'Missing or foreign file: {value}')
    for part in (path, *path.parents):
        if part == root:
            break
        if part.is_symlink():
            fail(f'Symlink is not accepted in a result/corpus path: {value}')
    return path


def inventory(root: Path, *, cache: bool = False) -> dict[str, str]:
    """Hash bytes and paths. Cache-internal symlinks are allowed and dereferenced."""
    root = root.resolve()
    files = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            if not cache or not path.resolve().is_relative_to(root) or path.is_dir():
                fail(f'Unexpected or escaping symlink: {path}')
        if path.is_file():
            files[path.relative_to(root).as_posix()] = sha256(path)
    if not files:
        fail(f'Empty corpus/cache: {root}')
    return files


def inventory_hash(files: dict) -> str:
    return text_hash(json.dumps(files, sort_keys=True, separators=(',', ':')))


def validate_manifest(manifest: dict, root: Path) -> dict[str, str]:
    object_value(manifest, 'query manifest')
    if manifest.get('schema_version') != 1 or manifest.get('split') != 'heldout-synthetic-v1':
        fail('Unsupported query manifest version/split')
    nonempty(manifest.get('name'), 'manifest name')
    if not isinstance(manifest.get('label_policy'), dict) or not manifest['label_policy']:
        fail('A readable label policy is required')
    files = inventory(root)
    if any(not re.fullmatch(r'docs/[a-z0-9_-]+\.md', path) for path in files):
        fail('Only the fixed corpus/docs/*.md sources belong in an index root')
    queries = manifest.get('queries')
    if not isinstance(queries, list) or not queries:
        fail('At least one query is required')
    seen, texts = set(), set()
    corpus_texts = [' '.join((root / path).read_text(encoding='utf-8').casefold().split())
                    for path in files]
    for query in queries:
        object_value(query, 'query')
        ident = nonempty(query.get('id'), 'query id')
        if not re.fullmatch(r'[A-Za-z0-9_-]+', ident) or ident in seen:
            fail(f'Invalid/duplicate query id: {ident}')
        seen.add(ident)
        nonempty(query.get('title'), 'query title')
        nonempty(query.get('language'), 'query language')
        query_text = nonempty(query.get('text'), 'query text')
        normalized = ' '.join(query_text.casefold().split())
        if normalized in texts:
            fail('Duplicate query text')
        texts.add(normalized)
        if any(normalized in document for document in corpus_texts):
            fail(f'Query text leaked into the searchable corpus: {ident}')
        tags = query.get('tags')
        if not isinstance(tags, list) or not tags or any(not isinstance(t, str) or not t for t in tags):
            fail('Every query needs nonempty tags')
        judgments = query.get('judgments')
        if not isinstance(judgments, list):
            fail('Judgments must be a list')
        judged = set()
        for row in judgments:
            object_value(row, 'judgment')
            path = row.get('path')
            if path not in files or path in judged:
                fail(f'Unknown/duplicate judgment source: {path}')
            judged.add(path)
            grade = integer(row.get('grade'), 'grade')
            if grade > 3:
                fail('Grades must be in 0..3')
            nonempty(row.get('rationale'), 'judgment rationale')
        if judged != files.keys():
            fail(f'All corpus sources must be explicitly judged for {ident}')
    return files


def score_query(query: dict, paths: list[str], k: int = K) -> dict:
    """Source gains use original chunk ranks; repeats get zero gain, never free slots."""
    grades = {row['path']: row['grade'] for row in query['judgments']}
    if any(path not in grades for path in paths):
        fail('Unjudged source cannot be silently treated as irrelevant')
    paths = paths[:k]
    relevant = {path for path, grade in grades.items() if grade >= 2}
    required = {path for path, grade in grades.items() if grade == 3}
    seen, gained = set(), []
    for path in paths:
        gained.append(grades[path] if path not in seen else 0)
        seen.add(path)
    dcg = sum((2 ** grade - 1) / math.log2(rank + 1)
              for rank, grade in enumerate(gained, 1))
    ideal = sum((2 ** grade - 1) / math.log2(rank + 1)
                for rank, grade in enumerate(sorted(grades.values(), reverse=True)[:k], 1))
    first = next((rank for rank, path in enumerate(paths, 1) if path in relevant), None)
    return {
        'source_ndcg_at_5': dcg / ideal if ideal else None,
        'source_hit_at_5': int(first is not None) if relevant else None,
        'source_reciprocal_rank_at_5': (1 / first if first else 0) if relevant else None,
        'source_recall_at_5': len(seen & relevant) / len(relevant) if relevant else None,
        'grade_3_source_recall_at_5': len(seen & required) / len(required) if required else None,
        'unique_sources_at_5': len(seen),
        'relevant_sources_at_5': len(seen & relevant),
        'grade_zero_sources_at_5': sum(grades[path] == 0 for path in seen),
        'returned_chunks_at_5': len(paths),
        'duplicate_slots_at_5': len(paths) - len(seen),
        'duplicate_fraction_at_5': (len(paths) - len(seen)) / len(paths) if paths else 0,
        'all_zero_judgments': not any(grades.values()),
    }


def aggregate(rows: list[dict]) -> dict:
    keys = [key for key in rows[0]['metrics'] if key != 'all_zero_judgments'] if rows else []
    return {
        'query_count': len(rows),
        'error_count': sum(row.get('error') is not None for row in rows),
        'means': {key: sum(values) / len(values) if values else None
                  for key in keys
                  for values in [[row['metrics'][key] for row in rows if row['metrics'][key] is not None]]},
        'denominators': {key: sum(row['metrics'][key] is not None for row in rows) for key in keys},
    }


def normalize_response(envelope: dict, returncode: int, query: dict,
                       workspace: Path, sources: dict) -> list[dict]:
    if not isinstance(envelope, dict):
        fail('Expected CLI envelope object')
    meta = object_value(envelope.get('meta'), 'response metadata')
    if meta.get('schema_version') != '1.0.0' or meta.get('query') != query['text']:
        fail('Wrong envelope schema/query; possible misassociated response')
    if meta.get('entity_type') != 'document':
        fail('Expected document result envelope')
    if envelope.get('type') != 'result' or envelope.get('error') is not None:
        fail('Partial/error envelope cannot be scored as successful retrieval')
    if (envelope.get('status') == 'not_found' and envelope.get('code') == 'NOT_FOUND'
            and returncode in (0, 1) and envelope.get('exit_code') == 1
            and envelope.get('data') in (None, [])):
        return []
    if (returncode != 0 or envelope.get('status') != 'success' or envelope.get('code') != 'OK'
            or envelope.get('exit_code') != 0):
        fail(f'Retrieval failed, not an empty result: {envelope.get("code")} / {returncode}')
    items = envelope.get('data')
    if (not isinstance(items, list) or len(items) > K
            or type(meta.get('count')) is not int or meta['count'] != len(items)):
        fail('Invalid result list/count/top-five budget')
    rows, chunk_ids = [], set()
    for item in items:
        if not isinstance(item, dict) or item.get('collection') != COLLECTION:
            fail('Foreign collection or malformed result')
        chunk_id = integer(item.get('chunk_id'), 'chunk_id', 1)
        if chunk_id in chunk_ids:
            fail('Repeated chunk identity in a single response')
        chunk_ids.add(chunk_id)
        score = item.get('similarity')
        if type(score) not in (int, float) or not math.isfinite(score) or not -1.000001 <= score <= 1.000001:
            fail('Expected finite cosine similarity in [-1, 1]')
        value = nonempty(item.get('source_path'), 'source_path')
        absolute = Path(value)
        if absolute.is_absolute():
            # macOS exposes temporary roots through /var -> /private/var.
            # Resolve only the workspace ancestor, preserving the relative
            # spelling so contained_file still rejects source-level symlinks.
            root = workspace.resolve()
            ancestor = next((parent for parent in absolute.parents
                             if parent.resolve() == root), None)
            if ancestor is None:
                fail(f'Foreign result path: {value}')
            value = absolute.relative_to(ancestor).as_posix()
        path = contained_file(workspace, value)
        relative = path.relative_to(workspace.resolve()).as_posix()
        if relative not in sources or sha256(path) != sources[relative]:
            fail(f'Unjudged or stale source: {relative}')
        bounds = item.get('byte_range')
        if not isinstance(bounds, list) or len(bounds) != 2:
            fail('Invalid byte range')
        start, end = (integer(value, 'byte coordinate') for value in bounds)
        content = path.read_bytes()
        if not 0 <= start < end <= len(content):
            fail('Out-of-bounds/empty source range')
        content[start:end].decode('utf-8')
        rows.append({'path': relative, 'chunk_id': chunk_id, 'byte_range': bounds,
                     'similarity': score})
    return rows


def validate_diagnostics(stats: dict, experiment: dict) -> dict:
    object_value(stats, 'diagnostics')
    if stats.get('name') != COLLECTION:
        fail('Diagnostics describe another collection')
    if integer(stats.get('file_count'), 'file_count') != len(experiment['sources']):
        fail('Indexed file count differs from the fixed corpus')
    count = integer(stats.get('chunk_count'), 'chunk_count', 1)
    embedding = stats.get('embedding_index')
    if not isinstance(embedding, dict):
        fail('Missing embedding diagnostics')
    nonempty(embedding.get('generation'), 'index generation')
    if integer(embedding.get('unembedded_chunks'), 'unembedded_chunks') != 0:
        fail('Missing vectors: cannot qualify semantic relevance')
    if integer(embedding.get('live_vectors'), 'live_vectors', 1) != count:
        fail('Vector count differs from chunk count')
    if integer(embedding.get('physical_vectors'), 'physical_vectors', 1) < count:
        fail('Physical vector count is incomplete')
    raw = nonempty(embedding.get('identity'), 'embedding identity')
    suffix = ';document-input=2'
    if not raw.endswith(suffix):
        fail('Unknown document input policy')
    identity = object_value(strict_json(raw[:-len(suffix)]), 'embedding identity')
    if (identity.get('backend') != 'local' or identity.get('endpoint_sha256') is not None
            or identity.get('model') != experiment['model']
            or identity.get('model_revision') != experiment['model_revision']):
        fail('Expected matching local model/revision from actual diagnostics')
    policy = identity.get('input_policy', '')
    match = re.fullmatch(r'complete-input-v2:huggingface-tokenizers-([^:]+):([0-9a-f]{64}):(\d+)', policy)
    if not match or int(match[3]) < 1:
        fail('Missing exact tokenizer/input budget in diagnostic identity')
    return {'identity': identity, 'document_input_version': 2,
            'tokenizer_version': match[1], 'normalized_tokenizer_sha256': match[2],
            'effective_max_input_tokens': int(match[3]), 'generation': embedding['generation'],
            'file_count': stats['file_count'], 'chunk_count': count,
            'live_vectors': embedding['live_vectors'], 'physical_vectors': embedding['physical_vectors'],
            'unembedded_chunks': embedding['unembedded_chunks']}


def settings_text(workspace: Path, model: str, revision: str) -> str:
    if model not in LOCAL_MODELS:
        fail('This focused evaluator supports AllMiniLML6V2 and MultilingualE5Small only')
    nonempty(revision, 'model revision')
    # json.dumps produces a valid TOML basic string for these explicit local values.
    return f'''version = 1
workspace_root = {json.dumps(str(workspace))}
index_path = {json.dumps(str(workspace / '.codanna/index'))}

[indexing]
parallelism = 2
show_progress = false

[semantic_search]
enabled = true
model = {json.dumps(model)}
model_revision = {json.dumps(revision)}
embedding_threads = 1

[documents]
enabled = true

[documents.defaults]
min_chunk_chars = 160
max_chunk_chars = 900
overlap_chars = 80

[documents.collections.{COLLECTION}]
paths = ["docs"]
patterns = ["**/*.md"]
'''


def prepare(out: Path, model: str, revision: str, manifest_path: Path = MANIFEST,
            corpus: Path = CORPUS) -> dict:
    nonempty(model, 'model')
    nonempty(revision, 'model revision')
    manifest = read_json(manifest_path)
    sources = validate_manifest(manifest, corpus)
    out = out.resolve()
    if out.is_relative_to(HERE.parents[3]):
        fail('Keep experiment outputs, indexes and models outside the checkout')
    out.mkdir(parents=True, exist_ok=False)
    workspace = out / 'workspace'
    shutil.copytree(corpus, workspace)
    shutil.copyfile(manifest_path, out / 'queries.json')
    config = out / 'settings.toml'
    config.write_text(settings_text(workspace, model, revision), encoding='utf-8')
    experiment = {'schema_version': 1, 'status': 'unrun', 'full_qualification': False,
                  'semantic_relevance_measured': False, 'model': model, 'model_revision': revision,
                  'source_root': 'workspace', 'sources': sources,
                  'corpus_sha256': inventory_hash(sources), 'queries_sha256': sha256(out / 'queries.json'),
                  'evaluator_sha256': sha256(HERE / 'evaluate.py'),
                  'settings_sha256': sha256(config), 'k': K,
                  'ranking': 'public_documents_search_selected',
                  'raw_cosine_comparison': RAW_COMPARISON}
    write_json(out / 'experiment.json', experiment)
    return experiment


def load_experiment(out: Path) -> tuple[dict, dict]:
    experiment = object_value(read_json(out / 'experiment.json'), 'experiment')
    if experiment.get('schema_version') != 1 or experiment.get('k') != K:
        fail('Unsupported experiment schema/cutoff')
    if experiment.get('source_root') != 'workspace' or experiment.get('ranking') != 'public_documents_search_selected':
        fail('Unsupported source root or ranking mode')
    if sha256(HERE / 'evaluate.py') != experiment.get('evaluator_sha256'):
        fail('Evaluator changed after preparation; prepare a new comparable run')
    if sha256(out / 'queries.json') != experiment.get('queries_sha256'):
        fail('Stale query manifest hash')
    if sha256(out / 'settings.toml') != experiment.get('settings_sha256'):
        fail('Settings changed after preparation')
    expected_settings = settings_text((out / 'workspace').resolve(), experiment['model'], experiment['model_revision'])
    if (out / 'settings.toml').read_text(encoding='utf-8') != expected_settings:
        fail('Only generated local-model settings are supported')
    manifest = read_json(out / 'queries.json')
    # Runtime metadata is outside the only searchable path, workspace/docs.
    sources = inventory(out / 'workspace' / 'docs')
    sources = {'docs/' + path: value for path, value in sources.items()}
    if sources != experiment.get('sources') or inventory_hash(sources) != experiment.get('corpus_sha256'):
        fail('Stale/foreign corpus bytes')
    validate_manifest(manifest, CORPUS)
    if sources != inventory(CORPUS) or sha256(out / 'queries.json') != sha256(MANIFEST):
        fail('Capture differs from this fixed evaluation dataset')
    return experiment, manifest


def isolated_env(home: Path) -> dict[str, str]:
    env = {key: os.environ[key] for key in ('PATH', 'SYSTEMROOT', 'WINDIR') if key in os.environ}
    env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / 'config'),
               XDG_CACHE_HOME=str(home / 'cache'), HF_HOME=str(home / '.codanna/models'),
               TMPDIR=str(home / 'tmp'), TEMP=str(home / 'tmp'), TMP=str(home / 'tmp'),
               HF_ENDPOINT='codanna-offline://cache', NO_COLOR='1',
               OMP_NUM_THREADS='2', RAYON_NUM_THREADS='2')
    return env


class Commands:
    """Bounded manual CLI capture; process tests use only local Python fixtures."""
    def __init__(self, out: Path, env: dict):
        self.out, self.env, self.history = out, env, []
        (out / 'raw').mkdir(exist_ok=False)

    def call(self, argv: list[str], cwd: Path, timeout: float = 120) -> dict:
        index = len(self.history)
        stdout, stderr = self.out / 'raw' / f'{index:03d}.json', self.out / 'raw' / f'{index:03d}.stderr'
        start = time.monotonic()
        problem = None
        with stdout.open('wb') as out, stderr.open('wb') as err, selectors.DefaultSelector() as ready:
            process = subprocess.Popen(argv, cwd=cwd, env=self.env, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       start_new_session=os.name == 'posix')
            streams = {process.stdout: ('stdout', out), process.stderr: ('stderr', err)}
            written = {'stdout': 0, 'stderr': 0}
            for stream in streams:
                ready.register(stream, selectors.EVENT_READ)

            def stop():
                try:
                    if os.name == 'posix':
                        os.killpg(process.pid, signal.SIGKILL)
                    else:
                        process.kill()
                except ProcessLookupError:
                    pass
                process.wait()

            try:
                while ready.get_map() and problem is None:
                    remaining = timeout - (time.monotonic() - start)
                    if remaining <= 0:
                        problem = 'timeout'
                        break
                    for key, _ in ready.select(min(remaining, 0.1)):
                        data = os.read(key.fd, 64 * 1024)
                        if not data:
                            ready.unregister(key.fileobj)
                            continue
                        name, destination = streams[key.fileobj]
                        allowance = MAX_JSON_BYTES - written[name]
                        destination.write(data[:allowance])
                        written[name] += min(len(data), allowance)
                        if len(data) > allowance:
                            problem = f'{name}_output_limit'
                            break
                if problem is None:
                    process.wait(timeout=max(0.001, timeout - (time.monotonic() - start)))
                else:
                    stop()
            except subprocess.TimeoutExpired:
                problem = 'timeout'
                stop()
            except BaseException:
                stop()
                raise
            finally:
                for stream in streams:
                    stream.close()
        entry = {'argv': argv, 'returncode': process.returncode, 'elapsed_seconds': time.monotonic() - start,
                 'stdout': stdout.relative_to(self.out).as_posix(), 'stdout_sha256': sha256(stdout),
                 'stderr': stderr.relative_to(self.out).as_posix(), 'stderr_sha256': sha256(stderr)}
        if problem:
            entry['error'] = problem
        self.history.append(entry)
        if problem:
            fail(f'Command stopped: {problem}; inspect retained bounded output')
        return entry


def captured_bytes(out: Path, entry: dict) -> bytes:
    object_value(entry, 'captured command')
    if type(entry.get('returncode')) is not int:
        fail('Captured command requires an integer return code')
    stdout = b''
    for key in ('stdout', 'stderr'):
        path = contained_file(out, entry[key])
        with path.open('rb') as stream:
            raw = stream.read(MAX_JSON_BYTES + 1)
        if len(raw) > MAX_JSON_BYTES:
            fail(f'Captured {key} exceeds the fixture budget')
        if not entry[key].startswith('raw/') or hashlib.sha256(raw).hexdigest() != entry.get(key + '_sha256'):
            fail('Stale or foreign captured output')
        if key == 'stdout':
            stdout = raw
    return stdout


def captured_json(out: Path, entry: dict):
    return strict_json(captured_bytes(out, entry).decode('utf-8'))


def require_command(entry: dict, argv: list[str]) -> None:
    object_value(entry, 'captured command')
    if entry.get('argv') != argv:
        fail('Captured command differs from the fixed CLI/config/query contract')


def cache_inventory(root: Path, model: str) -> dict[str, str]:
    """Select one already-cached snapshot, preserving the cache's relative layout.

    Do not copy/hash unrelated user cache entries or duplicate HF blob storage.
    Internal snapshot symlinks are materialized into the private cache as files.
    """
    if model not in LOCAL_MODELS:
        fail('Unsupported local model')
    root = root.resolve()
    repo, weight = LOCAL_MODELS[model]
    ref = root / repo / 'refs/main'
    if not ref.is_file() or not ref.resolve().is_relative_to(root):
        fail('Missing selected model refs/main; supply an existing complete cache, no download is initiated')
    if ref.stat().st_size != 40:
        fail('Cached refs/main must contain exactly one immutable 40-character snapshot id, without a newline')
    revision = ref.read_text(encoding='utf-8')
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        fail('Cached refs/main must contain exactly one immutable 40-character snapshot id, without a newline')
    snapshot = root / repo / 'snapshots' / revision
    assets = [ref] + [snapshot / name for name in
                     (weight, 'tokenizer.json', 'config.json', 'special_tokens_map.json', 'tokenizer_config.json')]
    for path in assets:
        if not path.is_file() or not path.resolve().is_relative_to(root):
            fail(f'Missing or escaping cached artifact: {path}; no download is initiated')
    return {path.relative_to(root).as_posix(): sha256(path) for path in sorted(assets)}


def collect(out: Path, binary: Path, cache: Path, allow_local_model: bool) -> dict:
    if not allow_local_model:
        fail('Manual inference requires --allow-local-model; checks and scoring need no model')
    experiment, manifest = load_experiment(out)
    binary, cache = binary.resolve(strict=True), cache.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail('Supply an already built executable Codanna binary')
    files = cache_inventory(cache, experiment['model'])
    if (out / 'capture.json').exists() or (out / 'home').exists() or (out / 'raw').exists():
        fail('Collection requires a fresh prepared run; prior evidence is never overwritten')
    home = out / 'home'
    home.mkdir()
    (home / 'tmp').mkdir()
    cached_copy = home / '.codanna/models'
    cached_copy.parent.mkdir()
    cached_copy.mkdir()
    for relative in files:
        target = cached_copy / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(cache / relative, target)
    if inventory(cached_copy) != files:
        fail('Cache changed while copying')
    commands = Commands(out, isolated_env(home))
    capture = {'schema_version': 1, 'origin': 'manual-local-cli', 'status': 'collecting',
               'started_at': datetime.now(timezone.utc).isoformat(),
               'experiment_sha256': sha256(out / 'experiment.json'),
               'binary': {'path': str(binary), 'sha256': sha256(binary)},
               'host': {'system': platform.system(), 'machine': platform.machine(),
                        'python': platform.python_version()},
               'cached_artifacts': files, 'cached_artifacts_sha256': inventory_hash(files),
               'responses': [], 'commands': commands.history}
    config = out / 'settings.toml'
    base = [str(binary), '--config', str(config), 'documents']
    try:
        capture['version'] = commands.call([str(binary), '--version'], out / 'workspace')
        if capture['version']['returncode'] != 0:
            fail('Codanna version command failed')
        capture['index'] = commands.call(base + ['index', '--collection', COLLECTION, '--no-progress'],
                                         out / 'workspace', timeout=600)
        if capture['index']['returncode'] != 0:
            fail('Document indexing failed; inspect retained raw output')
        capture['stats_before'] = commands.call(base + ['stats', COLLECTION, '--json'], out / 'workspace')
        if capture['stats_before']['returncode'] != 0:
            fail('Embedding diagnostics failed')
        validate_diagnostics(captured_json(out, capture['stats_before']), experiment)
        for query in manifest['queries']:
            entry = commands.call(base + ['search', query['text'], '--collection', COLLECTION,
                                          '--limit', str(K), '--json'], out / 'workspace')
            capture['responses'].append({'query_id': query['id'], 'query_sha256': text_hash(query['text']),
                                         'command': entry})
            # Stop after the first malformed/failed query; omitted queries are errors during scoring.
            normalize_response(captured_json(out, entry), entry['returncode'], query,
                               out / 'workspace', experiment['sources'])
        capture['stats_after'] = commands.call(base + ['stats', COLLECTION, '--json'], out / 'workspace')
        if capture['stats_after']['returncode'] != 0:
            fail('Final embedding diagnostics failed')
        if inventory(cached_copy) != files:
            fail('Model cache changed during inference; provenance is not stable')
        if sha256(binary) != capture['binary']['sha256']:
            fail('Binary changed during collection')
        capture['status'] = 'collected'
    except (OSError, ValueError, KeyError, TypeError) as error:
        capture.update(status='collection_error', error=str(error))
    finally:
        capture['finished_at'] = datetime.now(timezone.utc).isoformat()
        write_json(out / 'capture.json', capture)
    return capture


def evaluate(out: Path) -> dict:
    experiment, manifest = load_experiment(out)
    capture = object_value(read_json(out / 'capture.json'), 'capture')
    if capture.get('schema_version') != 1 or capture.get('origin') != 'manual-local-cli':
        fail('Expected explicit manual-local CLI capture')
    if capture.get('experiment_sha256') != sha256(out / 'experiment.json'):
        fail('Stale experiment identity')
    binary = object_value(capture.get('binary'), 'binary provenance')
    if not SHA256.fullmatch(binary.get('sha256', '')):
        fail('Missing binary SHA-256 provenance')
    files = capture.get('cached_artifacts')
    if not isinstance(files, dict) or not files or any(not SHA256.fullmatch(value) for value in files.values()):
        fail('Missing cached model artifact hashes')
    if inventory_hash(files) != capture.get('cached_artifacts_sha256'):
        fail('Stale model artifact inventory')
    # Actual copied files remain available in the original capture directory.
    if (inventory(out / 'home/.codanna/models') != files
            or cache_inventory(out / 'home/.codanna/models', experiment['model']) != files):
        fail('Stale cached model/tokenizer bytes')
    errors, diagnostics, version = [], None, None
    base = [nonempty(binary.get('path'), 'binary path'), '--config', str(out / 'settings.toml'), 'documents']
    try:
        require_command(capture['version'], [binary['path'], '--version'])
        require_command(capture['index'], base + ['index', '--collection', COLLECTION, '--no-progress'])
        if capture['version']['returncode'] != 0 or capture['index']['returncode'] != 0:
            fail('Version/index command did not complete successfully')
        version = nonempty(captured_bytes(out, capture['version']).decode('utf-8').strip(), 'binary version')
        captured_bytes(out, capture['index'])
    except (OSError, ValueError, KeyError, TypeError) as error:
        errors.append(f'Version/index evidence: {error}')
    # A present contradictory global identity/count invalidates all semantic scores.
    # Query failures may stop collection before final stats; retain their zero-score
    # denominator only when the initial global provenance remains valid.
    before = object_value(capture.get('stats_before'), 'initial embedding diagnostics command')
    require_command(before, base + ['stats', COLLECTION, '--json'])
    if before['returncode'] != 0:
        fail('Initial embedding diagnostics failed; model provenance is unavailable')
    diagnostics = validate_diagnostics(captured_json(out, before), experiment)
    after = capture.get('stats_after')
    if after is None and capture.get('status') == 'collection_error':
        errors.append('Final diagnostics unavailable after collection stopped')
    else:
        require_command(after, base + ['stats', COLLECTION, '--json'])
        if after['returncode'] != 0:
            captured_bytes(out, after)
            errors.append('Final embedding diagnostics command failed')
        else:
            other = validate_diagnostics(captured_json(out, after), experiment)
            if other != diagnostics:
                fail('Model/input/index generation or counts changed between queries')
    expected = {query['id']: query for query in manifest['queries']}
    responses = {}
    if not isinstance(capture.get('responses'), list):
        fail('Captured responses must be a list')
    for entry in capture['responses']:
        object_value(entry, 'captured response')
        ident = entry.get('query_id')
        if ident not in expected or ident in responses:
            fail('Unknown or duplicate captured query')
        responses[ident] = entry
    results = []
    for query in manifest['queries']:
        rows, error = [], None
        try:
            entry = responses[query['id']]
            if entry.get('query_sha256') != text_hash(query['text']):
                fail('Stale query text hash')
            command = entry['command']
            require_command(command, base + ['search', query['text'], '--collection', COLLECTION,
                                             '--limit', str(K), '--json'])
            rows = normalize_response(captured_json(out, command), command['returncode'], query,
                                      out / 'workspace', experiment['sources'])
        except (OSError, ValueError, KeyError, TypeError) as issue:
            error = f'{type(issue).__name__}: {issue}'
            errors.append(f'{query["id"]}: {error}')
        results.append({'id': query['id'], 'language': query['language'], 'tags': query['tags'],
                        'error': error, 'rows': rows,
                        'metrics': score_query(query, [row['path'] for row in rows])})
    if capture.get('status') != 'collected':
        errors.append(capture.get('error', 'Collection did not complete'))
    return {
        'schema_version': 1, 'status': 'measured' if not errors else 'incomplete',
        'full_qualification': False, 'semantic_relevance_measured': not errors,
        'qualification_limit': 'Small synthetic source-level judgments; independent human label '
                               'review, representative workloads, and historical manual cases remain pending.',
        'corpus_sha256': experiment['corpus_sha256'], 'queries_sha256': experiment['queries_sha256'],
        'evaluator_sha256': experiment['evaluator_sha256'],
        'binary': binary | {'version': version}, 'host': capture.get('host'),
        'model_artifacts_sha256': capture['cached_artifacts_sha256'],
        'model_artifact_paths': sorted(files),
        'revision_policy': 'Configured revision is an identity label, not a FastEmbed revision selector; '
                           'copied cache artifact hashes and snapshot/ref paths identify actual bytes.',
        'diagnostics': diagnostics, 'ranking': experiment['ranking'],
        'raw_cosine_comparison': RAW_COMPARISON,
        'judgment_coverage': {'judged_sources_per_query': len(experiment['sources']),
                             'all_results_judged': not errors,
                             'policy': 'reject incomplete judgments or unjudged retrieval'},
        'summary': aggregate(results),
        'by_language': {language: aggregate([row for row in results if row['language'] == language])
                        for language in sorted({row['language'] for row in results})},
        'by_tag': {tag: aggregate([row for row in results if tag in row['tags']])
                   for tag in sorted({tag for row in results for tag in row['tags']})},
        'results': results, 'errors': errors,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    sub.add_parser('check', help='Validate fixed corpus and judgments; no model/process calls')
    prep = sub.add_parser('prepare', help='Copy only corpus sources into a new disposable workspace')
    prep.add_argument('--out', required=True, type=Path)
    prep.add_argument('--model', required=True, choices=sorted(LOCAL_MODELS))
    prep.add_argument('--model-revision', required=True)
    manual = sub.add_parser('collect', help='MANUAL ONLY: use an existing local model cache')
    manual.add_argument('--run', required=True, type=Path)
    manual.add_argument('--codanna', required=True, type=Path)
    manual.add_argument('--cached-models', required=True, type=Path)
    manual.add_argument('--allow-local-model', action='store_true')
    score = sub.add_parser('score', help='Validate and score captured CLI JSON; no model/process calls')
    score.add_argument('--run', required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.action == 'check':
            manifest = read_json(MANIFEST)
            files = validate_manifest(manifest, CORPUS)
            text = {'status': 'unrun', 'full_qualification': False,
                    'semantic_relevance_measured': False, 'sources': len(files),
                    'queries': len(manifest['queries']), 'corpus_sha256': inventory_hash(files),
                    'queries_sha256': sha256(MANIFEST), 'metrics': None}
            frozen = read_json(HERE / 'status.json')
            if frozen != text:
                fail('Frozen status/hash manifest differs; review and explicitly update dataset version')
            print(json.dumps(text, indent=2))
        elif args.action == 'prepare':
            prepare(args.out, args.model, args.model_revision)
            print(f'Prepared an unrun experiment: {args.out.resolve()}')
        elif args.action == 'collect':
            capture = collect(args.run.resolve(), args.codanna, args.cached_models, args.allow_local_model)
            print(f'Capture status: {capture["status"]}')
            return 0 if capture['status'] == 'collected' else 2
        else:
            report = evaluate(args.run.resolve())
            write_json(args.run / 'report.json', report)
            print(json.dumps({'status': report['status'], 'full_qualification': False,
                              'summary': report['summary']}, indent=2))
            return 0 if report['status'] == 'measured' else 2
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        if args.action == 'score' and args.run.is_dir():
            write_json(args.run / 'report.json', {
                'schema_version': 1, 'status': 'invalid_capture', 'full_qualification': False,
                'semantic_relevance_measured': False, 'summary': None, 'errors': [str(error)],
            })
        print(f'Evaluation error: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())

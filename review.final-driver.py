import base64
import gzip
import hashlib
import json
import os
from pathlib import Path

root = Path(os.environ['RUNNER_TEMP'])
changed = set()
manifests = [
    ('review.final2.b64', 'd233403ccad2d028fb063d663b77169b79d41b4ff9c752b9311ea151986f9e33'),
    ('review.final3extra.b64', '31f25ffa7ec155553740b18df77353f780e982834e7bd9268285932dfb9b628b'),
    ('review.final4extra', '18b712b5c149a9a3d95db78978c5bc2c26cd7a22078a469f5dd9d5569094d962'),
]
for filename, digest in manifests:
    encoded = (root / filename).read_bytes() if filename.endswith('.b64') else b''.join(p.read_bytes() for p in sorted(root.glob(filename + '.*.b64')))
    if filename == 'review.final2.b64':
        repairs = [
            (b'2ZxVGcp35opW3OgqBwXFctNS6', b'2ZxVGcp35opW7OgqBwXFctNS6'),
            (b'tNS6cEK54NsYFlAS1OlMV54cSO', b'tNS6cEK54NsYHAS1OlMV54cSO'),
            (b'bHsmKVhnmVF4hVtJ7jEM5jLfc', b'bHsmKVhnmVF4VtJ7jEM5jLfc'),
            (b'hqtFqO1oMHZw4CHCFA/tnD9k', b'hqtFqO1oMHZw64CHCFA/tnD9k'),
        ]
    elif filename == 'review.final3extra.b64':
        repairs = [(b'MvSrIj00om5GAB', b'MvSr00om5GAB')]
    else:
        repairs = [(b'nma2XmelkUcz', b'nma2XmWelkUcz')]
    for before, after in repairs:
        assert encoded.count(before) == 1, before
        encoded = encoded.replace(before, after)
    raw = gzip.decompress(base64.b64decode(encoded))
    assert hashlib.sha256(raw).hexdigest() == digest, filename
    prepared = []
    for change in json.loads(raw):
        path = Path(change['path'])
        assert not path.is_absolute() and '..' not in path.parts and '.git' not in path.parts
        old = path.read_bytes() if path.exists() else b''
        if change['old'] is None:
            assert not path.exists(), str(path)
        else:
            assert hashlib.sha256(old).hexdigest() == change['old'], str(path)
        lines = old.decode().splitlines(keepends=True)
        for first, last, replacement in reversed(change['edits']):
            lines[first:last] = replacement.splitlines(keepends=True)
        new = ''.join(lines).encode()
        assert hashlib.sha256(new).hexdigest() == change['new'], str(path)
        prepared.append((path, new))
    for path, new in prepared:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(new)
        changed.add(path)
    print(f'Applied {filename}: {len(prepared)} verified files')
(root / 'changed-rust.txt').write_text('\n'.join(str(p) for p in sorted(changed) if p.suffix == '.rs'))

import base64
import gzip
import hashlib
import json
import os
import subprocess
from pathlib import Path

# Earlier manifests describe source before rustfmt; normalize before applying
# line edits against the exact formatted candidate previously inspected.
subprocess.run(['cargo', 'fmt', '--all'], check=True)
encoded = Path(os.environ['RUNNER_TEMP'], 'review.final-supplement.b64').read_bytes()
encoded = encoded.replace(b'i2mglA2', b'i2lglA2').replace(b'Wq9o2Rv', b'Wq9oRv')
raw = gzip.decompress(base64.b64decode(encoded, validate=True))
assert hashlib.sha256(raw).hexdigest() == '599e06c280df99849ecb2c048bc50b6e30f5db2b22b68a63b82fc88d56888d52'
prepared = []
for change in json.loads(raw):
    path = Path(change['path'])
    assert not path.is_absolute() and '..' not in path.parts and '.git' not in path.parts
    assert not path.is_symlink()
    old = path.read_bytes()
    assert hashlib.sha256(old).hexdigest() == change['old'], str(path)
    lines = old.decode().splitlines(keepends=True)
    for first, last, replacement in reversed(change['edits']):
        lines[first:last] = replacement.splitlines(keepends=True)
    new = ''.join(lines).encode()
    assert hashlib.sha256(new).hexdigest() == change['new'], str(path)
    prepared.append((path, new))
for path, data in prepared:
    path.write_bytes(data)
print(f'Applied {len(prepared)} verified final fixes')

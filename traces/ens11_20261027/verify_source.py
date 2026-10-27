#!/usr/bin/env python3
"""Verify the ENS11 tracked build-input inventory and content seal (no build or GPU)."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
args = parser.parse_args()
base = Path(__file__).resolve().parent
meta = json.loads((base / 'source-meta.json').read_text())
expected = json.loads((base / 'source-sha256.json').read_text())
paths = subprocess.check_output(
    ['git', 'ls-files', '-z', '--', *meta['scope']], cwd=args.root
).decode().split('\0')
actual = {
    p: hashlib.sha256((args.root / p).read_bytes()).hexdigest() for p in sorted(paths) if p
}
changed = sorted(p for p in actual.keys() | expected.keys() if actual.get(p) != expected.get(p))
if changed:
    raise SystemExit(f'Build inputs differ ({len(changed)}): {changed[:20]}')
raw = (json.dumps(actual, indent=2, sort_keys=True) + '\n').encode()
seal = hashlib.sha256(raw).hexdigest()
if seal != meta['seal_sha256']:
    raise SystemExit('Manifest encoding/seal mismatch')
print(f'OK: {len(actual)} build inputs, SHA256 {seal}')

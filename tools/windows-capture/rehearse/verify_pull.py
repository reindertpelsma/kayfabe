#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run on the trusted capture host: verify every pulled private file and summarize sidecars."""
import hashlib
import json
import re
from pathlib import Path, PurePosixPath
import sys


OUTPUT_NAME = re.compile(
    r"(?:(?:warm|startup[12])/(?:capture\.kfgspcap|cell\.json)|"
    r"(?:warm|startup[12])\.(?:stdout|stderr)\.txt|startup[12]\.stop|"
    r"(?:d3d|health)[12]\.json|(?:failure|remove|gsp-version)\.txt)"
)


def manifest_rows(root):
    source = Path(root) / 'manifest.json'
    if source.stat().st_size > 65536:
        raise ValueError('manifest too large')
    rows = json.loads(source.read_text(encoding='utf-8-sig'))
    if not isinstance(rows, list) or not rows or len(rows) > 64:
        raise ValueError('empty or malformed manifest')
    seen = set()
    for row in rows:
        name = row['path']
        pure = PurePosixPath(name)
        if (not name or pure.is_absolute() or '..' in pure.parts or '\\' in name
                or ':' in name or name in seen or name == 'manifest.json'):
            raise ValueError('unsafe or repeated manifest path')
        if not OUTPUT_NAME.fullmatch(name):
            raise ValueError('unsupported private output name')
        seen.add(name)
    return rows


def manifest_names(root):
    return [r['path'] for r in manifest_rows(root)]


def verify(root):
    root = Path(root).resolve()
    rows = manifest_rows(root)
    seen = set()
    total = 0
    for row in rows:
        name = row['path']
        pure = PurePosixPath(name)
        if (not name or pure.is_absolute() or '..' in pure.parts or '\\' in name
                or ':' in name or name in seen or name == 'manifest.json'):
            raise ValueError('unsafe or repeated manifest path')
        path = root.joinpath(*pure.parts)
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError('manifest escapes capture directory')
        data = path.read_bytes()
        if len(data) != row['bytes'] or hashlib.sha256(data).hexdigest() != row['sha256']:
            raise ValueError('hash/size mismatch: ' + name)
        seen.add(name)
        total += len(data)
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    if actual != seen | {'manifest.json'}:
        raise ValueError('unmanifested or missing file')
    cells = []
    for name in sorted(seen):
        if not name.endswith('/cell.json'):
            continue
        cell = json.loads((root / name).read_text(encoding='utf-8-sig'))
        cells.append({'cell': name.split('/')[0], 'capabilities': cell.get('capabilities', {}),
                      'discovery': cell['discovery'], 'stats': cell['stats'],
                      'capture': cell['capture']})
    return {'hashes_ok': True, 'files': len(seen), 'bytes': total, 'cells': cells}


def main():
    if len(sys.argv) == 3 and sys.argv[2] == '--names':
        print('\n'.join(manifest_names(sys.argv[1])))
        return 0
    result = verify(sys.argv[1])
    print(json.dumps(result, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())

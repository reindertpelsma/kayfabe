#!/usr/bin/env python3
"""Independent full-frame oracle: apply the KMS blob's DIRECT10 index to the
baseline RGB8888 pixels. This does not execute the CUDA implementation's source.
The tested NVKMS RGB path has identity input/FMT/OCSC and no alpha blending.
"""
import hashlib
import json
from pathlib import Path
import re
import sys
from PIL import Image


def main():
    root, out = map(Path, sys.argv[1:])
    manifest = json.loads((root / 'manifest.json').read_text())
    lut = {}
    for line in (root / 'sway-warm.log').read_text().splitlines():
        m = re.fullmatch(r'COLOR_LUT object=\d+ name=GAMMA_LUT index=(\d+) rgb=(\d+),(\d+),(\d+)', line)
        if m:
            index = int(m[1])
            if index in lut:
                raise ValueError('Ambiguous gamma tables')
            lut[index] = tuple(map(int, m.groups()[1:]))
    if set(lut) != set(range(1024)):
        raise ValueError('Need one complete 1024-entry KMS gamma table')
    before = Image.open(root / 'sway-before.ppm').convert('RGB')
    warm = Image.open(root / 'sway-warm.ppm').convert('RGB')
    restored = Image.open(root / 'sway-restored.ppm').convert('RGB')
    if before.size != warm.size or before.size != restored.size:
        raise ValueError('Mode changed')
    src, actual = before.tobytes(), warm.tobytes()
    expected = bytes(lut[value << 2][channel % 3] >> 8 for channel, value in enumerate(src))
    errors = [abs(a - b) for a, b in zip(expected, actual)]
    result = dict(schema='kayfabe-sdr-curve/1', source_revision=manifest['source_revision'],
                  qemu_sha256=manifest['qemu_sha256'], dimensions=before.size,
                  kms_blob_entries=len(lut), checked_components=len(errors),
                  mismatched_components=sum(e != 0 for e in errors),
                  max_error=max(errors), restored_identical=(src == restored.tobytes()),
                  output_changed=(src != actual),
                  expected_sha256=hashlib.sha256(expected).hexdigest(),
                  actual_sha256=hashlib.sha256(actual).hexdigest(),
                  oracle='OGKM GetLUTIndex: UNORM8 << 2; OLUT high 8 bits; actual KMS blob',
                  limitation='One opaque RGB8888 Linux SDR cell; no physical post-LUT capture')
    out.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    if result['mismatched_components'] or not result['restored_identical'] or not result['output_changed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()

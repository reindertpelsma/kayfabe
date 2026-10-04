#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Export a validated recorder snapshot as compressed text, never as an executable."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('input', type=Path)
p.add_argument('output', type=Path)
p.add_argument('--decoder-dir', type=Path, required=True)
a = p.parse_args()
sys.path.insert(0, str(a.decoder_dir))
import decode_rpctrace
blob = a.input.read_bytes()
header, records = decode_rpctrace.verify_and_parse(blob, expect_version='575.51.03')
meta = dict(kind='header', schema=1, source_bytes=len(blob), source_sha256=hashlib.sha256(blob).hexdigest(), recorder_header=header)
with gzip.open(a.output, 'wt') as f:
    f.write(json.dumps(meta, sort_keys=True) + '\n')
    for at in range(0, len(blob), 65536):
        f.write(json.dumps(dict(kind='chunk', offset=at, hex=blob[at:at+65536].hex())) + '\n')
print(json.dumps(meta, sort_keys=True))

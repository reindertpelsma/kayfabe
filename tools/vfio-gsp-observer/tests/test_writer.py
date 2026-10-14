#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Run the production writer using the exact configured QEMU compiler flags."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import struct
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('qemu_build', type=Path)
args = parser.parse_args()
build = args.qemu_build.resolve()
root = Path(__file__).resolve().parents[1]
row = next(r for r in json.loads((build / 'compile_commands.json').read_text())
           if r['file'].endswith('/hw/vfio/gsp-observer.c'))
flags = shlex.split(row['command'])
for option in ('-MD',):
    flags.remove(option)
for option in ('-MQ', '-MF', '-o', '-c'):
    pos = flags.index(option)
    del flags[pos:pos + 2]
file_header = struct.Struct('<4I2Q2I3Q')
record_header = struct.Struct('<4I2Q8I')
fields = ('magic', 'header_bytes', 'payload_bytes', 'direction', 'qpc', 'table_pa',
          'queue_sequence', 'rpc_sequence', 'rpc_function', 'rpc_result', 'flags',
          'missing_before', 'rpc_version', 'reserved')
with tempfile.TemporaryDirectory() as tmp:
    exe = Path(tmp) / 'writer-test'
    # Compile real QEMU memory predicates into independent sections so GC keeps
    # those called by the production plain-RAM guard without a whole VM link.
    memory = Path(tmp) / 'memory.o'
    qemu_source = (build / row['file']).resolve().parents[2]
    subprocess.run(flags + ['-ffunction-sections', '-fdata-sections', '-c',
        str(qemu_source / 'system/memory.c'), '-o', str(memory)], cwd=build, check=True)
    # This unit test supplies MemoryRegion state directly; suppress only QOM's
    # constructor registration so unused full-VM topology code can be collected.
    subprocess.run(['objcopy', '--remove-section=.init_array',
        '--remove-section=.rela.init_array', str(memory)], check=True)
    subprocess.run(flags + ['-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections',
        '-I' + str((build / row['file']).resolve().parent),
        '-I' + str(root / 'core'), str(root / 'tests/writer_test.c'),
        str(root / 'core/queue.c'), str(memory), '-Wl,--start-group', str(build / 'libqemuutil.a'),
        str(build / 'libqom.a'), '-Wl,--end-group',
        '-lglib-2.0', '-lpthread', '-o', str(exe)], cwd=build, check=True)
    output = Path(tmp) / 'trace.jsonl'
    subprocess.run([exe, 'full', output], check=True)
    digest = hashlib.sha256()
    observations = hashlib.sha256()
    count = size = 0
    with output.open() as stream:
        h = json.loads(next(stream))
        raw = file_header.pack(*(h[k] for k in ('magic', 'version', 'header_bytes',
            'record_header_bytes', 'qpc_frequency', 'started_qpc', 'flags', 'reserved')), *h['reserved2'])
        digest.update(raw); observations.update(raw); size += len(raw)
        for line in stream:
            row = json.loads(line)
            if row['kind'] == 'footer':
                assert row['records'] == count == 6500
                assert row['source_bytes'] == size
                assert row['source_sha256'] == digest.hexdigest()
                assert row['observations_sha256'] == observations.hexdigest()
                assert row['file_export_complete'] and not row['capture_complete']
                assert not row['driver_stats']['dropped']
                assert not stream.read()
                break
            payload = bytes.fromhex(row['payload_hex'])
            raw = record_header.pack(*(row[k] for k in fields)) + payload
            assert row['queue_sequence'] == count
            assert row['generation'] == 1 and row['trigger'] == 2
            assert len(payload) == 8192
            assert struct.unpack_from('<I', payload, 36)[0] == count
            checksum = 0
            for (word,) in struct.iter_unpack('<I', payload): checksum ^= word
            assert checksum == 0
            observations.update(struct.pack('<2Q', row['generation'], row['trigger']) + raw)
            digest.update(raw); size += len(raw); count += 1
        else:
            raise AssertionError('missing footer')
    subprocess.run([exe, 'failure', '/dev/full'], check=True)
    spec = importlib.util.spec_from_file_location('vfio_decode', root / 'decode.py')
    decoder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(decoder)
    assert len(decoder.parse_jsonl(output)['records']) == 6500
    # Corrupt metadata without changing the legacy KGWT hash, then truncate.
    corrupt = Path(tmp) / 'corrupt.jsonl'
    with output.open() as src, corrupt.open('w') as dst:
        for i, line in enumerate(src):
            dst.write(line.replace('"generation":1,', '"generation":2,') if i == 1 else line)
    try:
        decoder.parse_jsonl(corrupt)
    except decoder.InvalidTrace:
        pass
    else:
        raise AssertionError('corrupted observation metadata was accepted')
    with corrupt.open('r+b') as stream:
        stream.truncate(1024)
    try:
        decoder.parse_jsonl(corrupt)
    except (decoder.InvalidTrace, json.JSONDecodeError):
        pass
    else:
        raise AssertionError('truncated JSONL was accepted')
print('writer JSONL framing, sequence, checksum, SHA256 and footer verified')

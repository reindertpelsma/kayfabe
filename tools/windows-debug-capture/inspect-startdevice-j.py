#!/usr/bin/env python3
"""Bounded offline disassembly of the pinned Windows580.88 startup failure.
No dump input, runtime addresses, NVIDIA execution, or product constants.
"""
import argparse
import hashlib
import json
import struct
import subprocess
from pathlib import Path

SHA256 = '31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c'
# Reviewed research RVAs only. Fixed windows begin/end at instruction boundaries.
RANGES = {
    'l_window_constructor_call': (0x16e965e, 0x16e9698),
    'l_window_failure_assert': (0x16e97ef, 0x16e981d),
    'l_constructor_initial_guards': (0x16f0481, 0x16f04bd),
    'l_constructor_buffer_count_guard': (0x16f04bd, 0x16f0536),
    'l_constructor_success_and_guard_failure': (0x16f05e9, 0x16f0647),
    'ilut_window_selection': (0x16e8ddb, 0x16e8e11),
    'ilut_capb_descriptor_fields': (0x16e8e11, 0x16e8edc),
    'descriptor_instance_count_getter': (0x1fbb0, 0x1fbb7),
    'tmo_capd_descriptor_fields': (0x16e8f12, 0x16e8fd2),
    'tmo_first_descriptor_defaults': (0x16e858c, 0x16e85bc),
    'tmo_pointer_population_and_constructor': (0x16e96b5, 0x16e9755),
    'tmo_constructor_pointer_shape': (0x16f04bd, 0x16f0564),
    'm_second_constructor_failure': (0x16e97c1, 0x16e97ef),
    'descriptor_instance_count_origin': (0x169cbbf, 0x169cbdb),
    'leaf_preconditions': (0x169d3f0, 0x169d44e),
    'leaf_false_return': (0x169d822, 0x169d83d),
    'descriptor_sizes_and_calls': (0x16e935d, 0x16e93d1),
    'conditional_tmo_size': (0x16e8ef8, 0x16e8fd2),
    'capability_predicate': (0x208c0, 0x208e8),
    'capability_getter_a': (0x1ed30, 0x1ed48),
    'capability_getter_b': (0x1ed70, 0x1ed88),
    'entry_size_rounding': (0x16f0690, 0x16f06a1),
    'false_propagation_a': (0x16b4f17, 0x16b4f58),
    'false_propagation_b': (0x1615207, 0x1615294),
    'false_propagation_c': (0x1614ae5, 0x1614b3c),
    'false_propagation_d': (0x19598e1, 0x1959935),
    'status_branch': (0x196553f, 0x196554f),
    'status_assignment': (0x1965bc6, 0x1965bf4),
    'optional_hdacodec': (0x16158bf, 0x16158f0),
    'optional_parent_result': (0x1959363, 0x1959382),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver', type=Path, required=True)
    parser.add_argument('--disassemble', action='store_true')
    args = parser.parse_args()
    if args.driver.stat().st_size > 128 * 1024 * 1024:
        raise SystemExit('driver exceeds fixed analysis bound')
    data = args.driver.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise SystemExit('pinned trusted driver SHA256 mismatch')
    pe = struct.unpack_from('<I', data, 60)[0]
    assert data[pe:pe+4] == b'PE\0\0'
    optional = pe + 24
    assert struct.unpack_from('<H', data, optional)[0] == 0x20b
    image_base = struct.unpack_from('<Q', data, optional + 24)[0]
    image_size = struct.unpack_from('<I', data, optional + 56)[0]
    assert all(0 <= start < end <= image_size and end - start <= 256
               for start, end in RANGES.values())
    print(json.dumps({'driver_sha256': SHA256,
                      'ranges': {name: [hex(a), hex(b)] for name, (a,b) in RANGES.items()}},
                     indent=2))
    if args.disassemble:
        for name, (start, end) in RANGES.items():
            result = subprocess.run(
                ['objdump', '-d', '-Mintel', f'--adjust-vma=-{image_base}',
                 f'--start-address={start}', f'--stop-address={end}', str(args.driver)],
                check=True, capture_output=True, text=True, timeout=10)
            if len(result.stdout) > 65536:
                raise SystemExit('unexpected disassembly output size')
            print(f'\n{name}:')
            # Remove the local pathname header; all instruction addresses are RVAs.
            print('\n'.join(line for line in result.stdout.splitlines()
                            if 'file format' not in line))


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Strict TMO exercise gate. A running compositor is not a passing TMO test.

Run: check_tmo_stage.py <run-directory> <classes.tsv> <output.json>
The grayscale fixture requests a constant-zero intensity curve. Requires an
accepted real atomic commit, nonzero TMO method binding, changed black output,
byte-exact restoration and no scanout refusal. Missing support fails first.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

from PIL import Image


def read(path):
    with path.open('rb') as stream:
        data = stream.read(128 * 1024 * 1024 + 1)
    if len(data) > 128 * 1024 * 1024:
        raise ValueError('Experiment input exceeds fixed analysis bound')
    return data.decode()


def main():
    run, table, out = map(Path, sys.argv[1:])
    manifest = json.loads(read(run / 'manifest.json'))
    if not manifest.get('required_tmo') or not manifest.get('scene_grayscale'):
        raise ValueError('This is not the explicit grayscale TMO experiment')
    before_log = read(run / 'sway-before.log')
    warm_log = read(run / 'sway-warm.log')
    restored_log = read(run / 'sway-restored.log')
    trace = read(run / 'sway-warm-trace.log')
    full_trace = read(run / 'qemu.log')
    rows = {}
    for line in read(table).splitlines():
        fields = line.split('\t')
        if len(fields) >= 3:
            rows[fields[1]] = fields
    binding = int(rows['NVC67E_SET_CONTEXT_DMA_TMO_LUT'][2])
    control = int(rows['NVC67E_SET_TMO_CONTROL'][2])
    update = int(rows['NVC67E_UPDATE'][2])
    free = int(rows['NVC67E_FREE'][2])
    size_high, size_low = map(int, rows['NVC67E_SET_TMO_CONTROL_SIZE'][2:4])
    methods = re.findall(r'METHOD chn=(\d+) kind=(\w+) method=(0x[0-9a-f]+) '
                         r'data=(0x[0-9a-f]+) remaining=(\d+)', trace)
    binds = [(chn, data) for chn, kind, method, data, _ in methods
             if kind == 'Window' and int(method, 16) == binding and int(data, 16)]
    controls = [(chn, data) for chn, kind, method, data, _ in methods
                if kind == 'Window' and int(method, 16) == control]
    assembled, armed = {}, {}
    for chn, kind, method, data, _ in methods:
        if kind != 'Window':
            continue
        if int(method, 16) == free:
            assembled.pop(chn, None)
            armed.pop(chn, None)
            continue
        handle, word = assembled.get(chn, (0, 0))
        if int(method, 16) == binding:
            handle = int(data, 16)
        if int(method, 16) == control:
            word = int(data, 16)
        assembled[chn] = handle, word
        if int(method, 16) == update:
            size = (word >> size_low) & ((1 << (size_high - size_low + 1)) - 1)
            armed[chn] = bool(handle and size == 1029)
    atomic = re.findall(r'TMO_TEST REQUEST plane=(\d+) prop=(\d+) blob=(\d+) '
                        r'flags=(\d+) test_only=(\d+) rc=(-?\d+) errno=(\d+)', restored_log)
    applied = [row for row in atomic if int(row[2]) and row[4] == '0' and row[5] == '0']
    missing = bool(re.search(r'TMO_TEST MISSING_TMO_LUT\b', warm_log + restored_log))
    images = []
    captures = {}
    for name in ('sway-before', 'sway-warm', 'sway-restored'):
        with Image.open(run / (name + '.ppm')) as im:
            if im.size != (1920, 1080):
                raise ValueError('Unexpected mode; do not reinterpret the fixture')
            pixels = im.convert('RGB').tobytes()
        images.append(pixels)
        captures[name] = hashlib.sha256(pixels).hexdigest()
    before, warm, restored = images
    gray = all(before[i] == before[i+1] == before[i+2] for i in range(0, len(before), 3))
    checks = dict(
        baseline_gpu_scene_ready=('WL_SCENE_READY' in before_log and
                                  'WL_SCENE_RENDERER NVIDIA' in before_log),
        request_adapter_armed=('TMO_TEST armed' in before_log),
        grayscale_nonzero_baseline=(gray and any(before)),
        tmo_property_available=not missing,
        real_non_test_atomic_accepted=bool(applied),
        nonzero_tmo_method_binding=bool(binds),
        tmo_control_programmed=bool(controls),
        nonzero_tmo_binding_armed_at_capture=any(armed.values()),
        trace_budget_not_exhausted=bool(methods) and int(methods[-1][4]) > 0,
        warm_scene_frame_callback=('WL_SCENE_READY' in warm_log),
        output_changed=(before != warm),
        zero_curve_output_black=not any(warm),
        restored_identical=(before == restored),
        restored_scene_ready=('WL_SCENE_READY' in restored_log),
        no_scanout_refusal=('SCANOUT REFUSED' not in full_trace and 'scanout STOPPED' not in full_trace),
    )
    passed = all(checks.values())
    result = dict(schema='kayfabe-linux-tmo-stage/1', source_revision=manifest['source_revision'],
                  qemu_sha256=manifest['qemu_sha256'], harness_sha256=manifest['harness_sha256'],
                  checks=checks, passed=passed, verdict='PASS' if passed else 'FAIL',
                  failure_reasons=[name for name, value in checks.items() if not value],
                  nonzero_tmo_bindings=binds, tmo_control_words=controls,
                  armed_tmo_windows=armed,
                  accepted_real_atomics=len(applied), screenshot_rgb_sha256=captures,
                  trace_method_records=len(methods),
                  trace_budget_remaining=int(methods[-1][4]) if methods else None,
                  missing_property_request_blocked_by_guest_adapter=missing,
                  limitations=['One grayscale zero-intensity fixture; no general TMO/HDR parity claim.',
                               'A nonzero binding alone does not prove processing.',
                               'The first missing-property gate rejects before kernel submission.'])
    out.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()

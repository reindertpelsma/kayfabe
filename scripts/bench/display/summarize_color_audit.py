#!/usr/bin/env python3
"""Compare the fixed scene and color requests; success replies alone are not a pass.

Uses Pillow for screenshots and the committed generated class TSV as vocabulary.
Run: summarize_color_audit.py <run-directory> <classes.tsv> <output.json>
"""
import collections
import hashlib
import json
from pathlib import Path
import re
import sys

from PIL import Image


def fnv(data):
    h = 1469598103934665603
    for b in data:
        h = ((h ^ b) * 1099511628211) & ((1 << 64) - 1)
    return f'{h:016x}'


def main():
    run, table, out = map(Path, sys.argv[1:])
    manifest = json.loads((run/'manifest.json').read_text())
    events = json.loads((run/'events.json').read_text())
    shots = {}
    scene_hash = re.search(r'WL_SCENE_HASH ([0-9a-f]+)', (run/'sway-before.log').read_text())[1]
    for name in ['sway-before', 'sway-warm', 'sway-restored']:
        path = run/(name+'.ppm')
        im = Image.open(path).convert('RGB')
        # The fixed 512x384 surface is centered by this Sway config, with no border.
        if im.size != (1920, 1080):
            raise ValueError('Unexpected output dimensions; do not guess the scene rectangle')
        scene = im.crop((704, 348, 1216, 732)).convert('RGBA')
        scene = scene.transpose(Image.Transpose.FLIP_TOP_BOTTOM)
        h = fnv(scene.tobytes())
        shots[name] = dict(sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                           scene_rgba_fnv=h, scene_matches_client=(h == scene_hash))
        im.save(run/(name+'.png'))
    rows = {}
    for line in table.read_text().splitlines():
        f = line.split('\t')
        if len(f) >= 3: rows[f[1]] = f
    selected = []
    for name in ['SET_ILUT_CONTROL', 'SET_CONTEXT_DMA_ILUT', 'SET_OFFSET_ILUT',
                 'SET_TMO_CONTROL', 'SET_CONTEXT_DMA_TMO_LUT', 'SET_OFFSET_TMO_LUT']:
        key = 'NVC67E_'+name
        selected.append(('Window', int(rows[key][2]), key))
    for name in ['HEAD_SET_OLUT_CONTROL', 'HEAD_SET_CONTEXT_DMA_OLUT', 'HEAD_SET_OFFSET_OLUT']:
        key = 'NVC77D_'+name
        row = rows[key]
        for head in range(4):
            selected.append(('Core', int(row[2])+head*int(row[3]), key+f'({head})'))
    methods = re.findall(r'METHOD chn=(\d+) kind=(\w+) method=(0x[0-9a-f]+) '
                         r'data=(0x[0-9a-f]+) remaining=(\d+)', (run/'qemu.log').read_text())
    colors = {}
    for kind, offset, name in selected:
        counts = collections.Counter(data for _, k, m, data, _ in methods
                                     if k == kind and int(m, 16) == offset)
        if counts: colors[name] = dict(counts)
    def gamma(name):
        text = (run/(name+'.log')).read_text()
        return re.findall(r'name=GAMMA_LUT value=\d+(?: bytes=\d+ entries=\d+ '
                          r'midpoint=[\d,]+ endpoint=[\d,]+)?', text)
    warm = (run/'sway-warm.log').read_text()
    result = dict(schema='kayfabe-linux-color-audit/1', manifest=manifest,
                  events=events, client_rgba_fnv=scene_hash, screenshots=shots,
                  all_three_outputs_identical=len({s['sha256'] for s in shots.values()}) == 1,
                  gamma_before=gamma('sway-before'), gamma_warm=gamma('sway-warm'),
                  gamma_restored=gamma('sway-restored'),
                  protocol_failure_event=bool(re.search(r'zwlr_gamma_control_v1@\d+\.failed\(', warm)),
                  injected_kms_failure=('COLOR_FAULT reject GAMMA_LUT' in warm),
                  generated_table_sha256=hashlib.sha256(table.read_bytes()).hexdigest(),
                  color_method_value_counts=colors, method_records=len(methods),
                  trace_budget_remaining=int(methods[-1][4]) if methods else None,
                  limits=['One AD104/580.159.04 guest cell; SDR fixed XRGB scene.',
                          'Native shader/client control and KMS readback do not capture physical post-LUT pixels.',
                          'Warm control tests output gamma, not arbitrary input LUT/HDR transformations.'])
    out.write_text(json.dumps(result, indent=2)+'\n')


if __name__ == '__main__':
    main()

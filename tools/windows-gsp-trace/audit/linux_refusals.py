#!/usr/bin/env python3
"""Classify the original 68 Linux-counterexample IDs by observed guest refusal."""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re

NOTES = {
    '0x0073012c': 'VRR notification: TellRMAboutVrrHead logs a warning and continues (OGKM 580.65.06 nvkms-vrr.c:418-423). VRR correctness was not tested.',
    '0x00731144': 'ELD audio capabilities: nvkms-hdmi.c:1147-1168 logs the failure and continues audio-power cleanup. Audio setup may be lost; not a tested audio pass.',
    '0x2080012b': 'Golden-image kernel channel in this boot has no twin. Prior v3 refusal audit documents lazy-context fallback for NOT_SUPPORTED; this does not justify refusing user-channel promotion.',
    '0x20800102': 'Selector-dependent: two served requests and one refusal. Log names UnmeasuredForwardedIndex 35. Other selectors, including Windows requests, need separate treatment.',
    '0x20800a38': 'FECS tracing: fecs_event_list.c:1527-1535 returns from tracing setup on failure. The tracing feature is affected; this boot continues.',
    '0x20800aff': 'Shared-data polling: gpu_user_shared_data.c:369-378 propagates the failure and does not update the requested polling mask. This is not a successful no-op; this boot still initializes.',
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('catalogue', type=Path)
    ap.add_argument('baseline', type=Path)
    ap.add_argument('output', type=Path)
    args = ap.parse_args()
    rows = [r for r in json.loads(args.catalogue.read_text())['controls'] if r['linux']]
    assert len(rows) == 68
    qemu = args.baseline / 'run_lin_b1t_580.65.06_qemu.trimmed.log'
    refused = collections.defaultdict(list)
    for line, text in enumerate(qemu.read_text().splitlines(), 1):
        m = re.search(r'GSP REFUSED fn76/(0x[0-9a-f]+)=(0x[0-9a-f]+)', text)
        if m:
            refused[f'0x{int(m[1],16):08x}'].append(dict(line=line, status=m[2]))
    result = []
    for r in rows:
        guest = r['linux'].get('ga106_linux5806506_to_kayfabe')
        status = guest['statuses'] if guest else {}
        ledger = refused.get(r['id'], [])
        if ledger:
            assert {x['status'] for x in ledger} == {'0x56'}
            category = 'refused; initialization survived in this boot'
        elif guest:
            assert set(status) == {'0x0'}
            category = 'served here; rejection effect untested'
        else:
            category = 'native Linux only; guest rejection effect untested'
        note = NOTES.get(r['id'])
        if note is None:
            note = ('Per-call feature loss not isolated; no global GPU-init failure in this boot. '
                    + ('Semantics remain unresolved.' if r['preferred_source'] is None else 'The public meaning does not prove the userspace caller ignores failure.')
                    if ledger else 'No refusal experiment for this call/selector is present in this baseline.')
        result.append(dict(id=r['id'], category=category, guest_trace_results=status,
                           refusal_ledger=ledger, consequence=note,
                           description=r['description'], guest_evidence=guest))
    counts = dict(collections.Counter(r['category'] for r in result))
    inputs = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(args.baseline.iterdir()) if p.is_file()}
    report = dict(schema=1, implementation='b98bdbec', guest_driver='580.65.06', gpu='GA106',
                  source_baseline_commit='c50fad9ac485f53d45d4ea77a21cb7206267c65a',
                  input_sha256=inputs, counts=counts, controls=result)
    args.output.mkdir(exist_ok=True, parents=True)
    (args.output / 'refusals.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    lines = ['# Consequences of rejecting Linux-observed Windows control IDs', '',
             '**STATUS: RESEARCH, 2026-10-04. Existing-run audit; no new failure injection.**', '',
             'The original 68 Linux-counterexample IDs split into **35 observed refused**, **15 served in the guest**, and **18 seen only in native Linux samples**. `result=none` alone is not a returned status: the refusal ledger independently confirms `0x56` for all 34 such IDs, plus one selector of GPU_GET_INFO_V2.', '',
             'Baseline: NVIDIA Linux 580.65.06 on GA106, Kayfabe binary `b98bdbec`, retained in `v3-windows` at `c50fad9a`. The same captured boot reaches SMI_RC=0 and display handoff with black_frames=0. Its display probe build **failed** and modetest reported no connected output: this is not a complete display-feature pass. Separate saved CUDA ladder boots of the same build pass cup2/cup3/cup8; they do not independently correlate each of these 35 refusals to a CUDA call.', '',
             'The result proves survival of the observed initialization path, not harmlessness across workloads, dies, versions or Windows. The 15 successful IDs were not deliberately rejected. For the 18 native-only IDs there is no corresponding guest response here. Unknown feature effects stay unknown.', '',
             'The prior [v3 refusal audit](../../../../../../docs/design/V3_REFUSAL_AUDIT.md) is essential context: refusing MC_SERVICE_INTERRUPTS once ended waits early; a preemption-mode refusal broke one Vulkan application despite other Vulkan tests passing; a missing SM-issue-rate query was fatal on Blackwell but unreached on GA102. These are separate examples, not newly demonstrated failures among all 68 rows.', '',
             '[Machine-readable evidence and hashes](refusals.json). Baseline logs are under [baseline/](baseline/). Source interpretations below use OGKM 580.65.06 at `307159f2623d3bf45feb9177bd2da52ffbc5ddf9`; the golden-channel fallback additionally has the historical audit above.', '',
             '| ID | Observed guest result | Consequence / limit |', '|---|---|---|']
    for r in result:
        statuses = ', '.join(f'{k}×{v}' for k,v in r['guest_trace_results'].items()) or 'not observed'
        if r['refusal_ledger']:
            statuses += '; posted 0x56 (ledger line ' + str(r['refusal_ledger'][0]['line']) + ')'
        lines.append('| `' + r['id'] + '` | ' + statuses + ' | ' + r['consequence'] + ' |')
    (args.output / 'README.md').write_text('\n'.join(lines) + '\n')
    print(json.dumps(counts, sort_keys=True))


if __name__ == '__main__':
    main()

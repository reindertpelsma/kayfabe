#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_compare.py DIR kayfabe:k1,k2,k3 vfio:v1,v2,v3 [...]: side-by-side table (median over captures, [min..max]) from DIR/TAG/TAG_metrics.json."""
import json, sys, statistics as S
d = sys.argv[1]
sides = []
for a in sys.argv[2:]:
    name, tags = a.split(':'); sides.append((name, [json.load(open(f'{d}/{t}/{t}_metrics.json')) for t in tags.split(',')]))
def get(m, path):
    for k in path.split('.'):
        if m is None: return None
        m = m.get(k) if isinstance(m, dict) else None
    return m if isinstance(m, (int, float)) else None
ROWS = [
 ('FRAME CADENCE (ETW present packets, steady window)', None),
 ('fps', 'fps'), ('frame ms median', 'frame_ms.med'), ('frame ms mean', 'frame_ms.mean'), ('frame ms p95', 'frame_ms.p95'), ('frame ms p99', 'frame_ms.p99'), ('frame ms stdev', 'frame_ms.stdev'),
 ('frames in flight (mean)', 'frames_in_flight_mean'), ('present pkt 178->180 ms median', 'present_pkt_lifetime_ms.med'),
 ('PRESENTMON (same window rule)', None),
 ('PM MsBetweenPresents median', 'pm.MsBetweenPresents.med'), ('PM MsBetweenPresents p99', 'pm.MsBetweenPresents.p99'), ('PM MsCPUBusy mean', 'pm.MsCPUBusy.mean'), ('PM MsCPUWait mean', 'pm.MsCPUWait.mean'),
 ('PM MsInPresentAPI mean', 'pm.MsInPresentAPI.mean'), ('PM MsGPUTime mean', 'pm.MsGPUTime.mean'), ('PM MsGPUBusy mean', 'pm.MsGPUBusy.mean'), ('PM MsGPUWait mean', 'pm.MsGPUWait.mean'), ('PM MsGPULatency median', 'pm.MsGPULatency.med'),
 ('PACKETS (target heavy queues)', None),
 ('packets/frame (heavy queues)', 'packets_per_frame_target_heavy'),
 ('Q0 packets/s', 'queues.Q0.packets_per_s'), ('Q0 svc ms median', 'queues.Q0.service_ms.med'), ('Q0 svc ms p95', 'queues.Q0.service_ms.p95'), ('Q0 svc ms p99', 'queues.Q0.service_ms.p99'), ('Q0 svc ms mean', 'queues.Q0.service_ms.mean'),
 ('Q1 svc ms mean', 'queues.Q1.service_ms.mean'), ('Q2 svc ms mean', 'queues.Q2.service_ms.mean'),
 ('Q0 svc sum ms/s (queue occupancy)', 'queues.Q0.svc_sum_ms_per_s'), ('Q1 svc sum ms/s', 'queues.Q1.svc_sum_ms_per_s'), ('Q2 svc sum ms/s', 'queues.Q2.svc_sum_ms_per_s'),
 ('Q0 hw outstanding 450->451 ms median', 'queues.Q0.hw_outstanding_450_451_ms.med'), ('Q0 hw outstanding p95', 'queues.Q0.hw_outstanding_450_451_ms.p95'),
 ('Q0 sw queue 178->450 ms median', 'queues.Q0.sw_queue_178_450_ms.med'), ('Q0 sw queue p95', 'queues.Q0.sw_queue_178_450_ms.p95'),
 ('Q0 retire 451->180 ms median', 'queues.Q0.retire_451_180_ms.med'), ('Q0 packet 178->180 ms median', 'queues.Q0.lifetime_178_180_ms.med'),
 ('other-target packets/s', 'queues.target_other.packets_per_s'), ('other-target svc ms mean', 'queues.target_other.service_ms.mean'),
 ('GPU-VISIBLE BUSY / IDLE (all hw queues, all processes)', None),
 ('hw outstanding union frac', 'hw_outstanding_union_frac'), ('hw service union frac', 'hw_service_union_frac'),
 ('idle gaps /s', 'idle_gaps.per_s'), ('idle total frac', 'idle_gaps.total_frac'), ('idle gap ms median', 'idle_gaps.dur_ms.med'), ('idle gap ms p95', 'idle_gaps.dur_ms.p95'),
 ('idle ms/frame (no queue has hw work)', 'per_frame_ms.idle_no_hw_work'),
 ('gap pre-submit ms/frame (CPU side)', 'gap_attribution.pre_ms_per_frame'), ('gap post-submit ms/frame (178->450)', 'gap_attribution.post_ms_per_frame'),
 ('gap: next pkt already submitted frac', 'gap_attribution.next_packet_already_submitted_frac'),
 ('Q0 empty frac', 'queue_starvation.Q0.queue_empty_frac'), ('Q1 empty frac', 'queue_starvation.Q1.queue_empty_frac'), ('Q2 empty frac', 'queue_starvation.Q2.queue_empty_frac'),
 ('QUEUE DEPTH', None),
 ('hw outstanding depth mean (all queues)', 'hw_depth_all_mean'), ('sw-queued depth mean (178..450, heavy queues)', 'sw_queued_mean'),
 ('SOFTWARE DEPENDENCY PATH (heavy queues)', None),
 ('unwaits / frame', 'unwait_path.unwaits_per_frame'), ('last completion -> 238 unwait ms median', 'unwait_path.done_to_unwait_ms.med'), ('last completion -> 238 unwait p95', 'unwait_path.done_to_unwait_ms.p95'),
 ('238 unwait -> next 450 same queue ms median', 'unwait_path.unwait_to_hwsubmit_ms.med'), ('238 -> 450 p95', 'unwait_path.unwait_to_hwsubmit_ms.p95'),
]
names = [n for n, _ in sides]
print('| metric | ' + ' | '.join(f'{n} (n={len(ms)})' for n, ms in sides) + ' | ratio ' + names[0] + '/' + names[-1] + ' |')
print('|---|' + '---|' * (len(sides) + 1))
for label, path in ROWS:
    if path is None: print(f'| **{label}** |' + ' |' * (len(sides) + 1)); continue
    cells = []; meds = []
    for n, ms in sides:
        v = [get(m, path) for m in ms]; v = [x for x in v if x is not None]
        if not v: cells.append('-'); meds.append(None); continue
        md = S.median(v); meds.append(md)
        cells.append(f'{md:.4g} [{min(v):.4g}..{max(v):.4g}]')
    ratio = f'{meds[0] / meds[-1]:.3g}' if meds[0] is not None and meds[-1] not in (None, 0) else '-'
    print(f'| {label} | ' + ' | '.join(cells) + f' | {ratio} |')

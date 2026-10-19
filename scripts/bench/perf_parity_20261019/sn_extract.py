#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sn_extract.py DIR TAG: stream DIR/TAG_dxg.csv (tracerpt CSV of the DxgKrnl ETL) into DIR/TAG_ev.pkl: the scheduler events the analysis uses,
as {event_id: [(t_100ns_filetime, pid, tid, cpu, fields...)]}. User-data field order is the guest manifest's (Get-WinEvent on the same etl,
2026-10-18):
 178 QueuePacket start : hContext,PacketType,SubmitSequence,DmaBufferSize,AllocListSize,PatchListSize,bPresent,hDmaBuffer,pQueuePacket,ProgressFenceValue
 180 QueuePacket stop  : hContext,PacketType,SubmitSequence,bPreempted,bTimeouted,pQueuePacket
 179 QueuePacket info  : hContext,PacketType,SubmitSequence            (signal / paging packets)
 244/245 wait/signal QP: hContext,SubmitSequence,Flags,...,FenceValue,pQueuePacket,...
 450 HwQueue submit    : hHwQueue(==hContext),ProgressFenceValue,pDmaBuffer,ntStatus,NumberOfQueuedPendingFlip
 451 HwQueue progress  : hHwQueue,ProgressFenceValue
 238 UnwaitQueuePacket : QueuePacket,hContext      300 UnwaitCpuWaiter : Event,WaitConditionCount,FenceValues,...
 552 signal sync obj   : ...,pCpuEvent,pGuestCpuEvent          17 VSyncDPC  : ...,FrameNumber,FrameQPCTime,...
 175/176/177 DmaPacket (paging only here): hContext,...,PacketType,...
"""
import csv, pickle, sys, collections
KEEP = {'105', '106', '107', '108', '17', '18', '19', '22', '175', '176', '177', '178', '179', '180', '181', '238', '244', '245', '300', '450', '451', '552', '550', '295', '294'}
d, tag = sys.argv[1], sys.argv[2]
ev = collections.defaultdict(list)
profiler_order = []
n = 0
with open(f'{d}/{tag}_dxg.csv', encoding='utf-8', errors='replace', newline='') as f:
    r = csv.reader(f, skipinitialspace=True); next(r)
    for row in r:
        if len(row) < 18: continue
        i = row[2].strip()
        n += 1
        if i not in KEEP: continue
        try: t = int(row[16])
        except ValueError: continue
        ev[i].append((t, int(row[9], 16), int(row[10], 16), int(row[11]), *[x.strip() for x in row[19:]]))
        if i in ('105', '106'):
            profiler_order.append((i == '105', len(ev[i]) - 1))
ev['_p1_profiler_order'] = profiler_order
pickle.dump((dict(ev), n), open(f'{d}/{tag}_ev.pkl', 'wb'))
print('rows', n, {k: len(v) for k, v in sorted(ev.items(), key=lambda kv: -len(kv[1]))})

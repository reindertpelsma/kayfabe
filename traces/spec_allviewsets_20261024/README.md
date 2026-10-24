# SPECviewperf 15 all-viewsets correctness campaign on candidate 6d, 2026-10-24

**STATUS: LIVE, 2026-10-24.** Correctness (run to completion), not performance. Host: the owner's RTX 4070 (AD104) box,
host driver 595.91.07; Windows 11 guest, NVIDIA 580.88, 8 vCPU, 12 GiB, 8192 MiB kayfabe store, FullHD, 1 iteration per run.
Labels: MEASURED (named run) / CODE READ / INFERRED. No scores are published in this file; the (nc) numbers stay in the
operator's private report (docs/SPEC_RESULTS.md rules). SPEC and SPECviewperf are trademarks of the Standard Performance
Evaluation Corporation, https://www.spec.org. The benchmark copy is the owner's private installation inside the guest image; nothing of it is in this repository.

## Revision under test

`integration/prod-candidate-6d-20261023` @ `714ec29d3fdca0a013d9d478755346b0f1af6fa7` (PR #99 head). By `git merge-base --is-ancestor`:
contains #76 descriptor Phase 1 (`468a35c9d`), #73 SA-02 (merged at `7cbacd778`), #74 SA-03/VA-2 (`e6a0d2b81`), #90 CE class at birth
(`9490d7227`), #92 colour (`e636236cf`), #102 compile fix (`01a171e63`), #113 review fixes (`714ec29d3`). Not contained: Phase 2 #79.
Binary: Q4's farm build of that exact clean revision (`kf3-bins/714ec29d`, md5 `66b93d89f85fd19f2df881d4bc356e38`).
Flags: the owner's production set of run 4501 (22 flags, `KF3_PROF` removed) + `KF3_DIAG_PROGRESS=1000`, `WIN_DROP=KF3_TIMER_MAP`.

## Results (MEASURED, one viewset per fresh boot, fresh CoW overlay each)

| run | viewset | subtests completed | guest 153/4101 | host new Xid | RM unloads | Budget / SCAN_BUDGET / NoCeObject / poisoned | large Pointer descriptors (count, max bytes) | verdict |
|---|---|---|---|---|---|---|---|---|
| 5201 | enscape-01 | 10/10 | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 65,536 | PASS (first ever complete) |
| 5202 | maya-07 | 7/7 | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 129,768 | PASS (first ever run) |
| 5203 | medical-04 | 10/10 | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 377,884 | PASS (first ever run) |
| 5204 | solidworks-08 | 8/8 | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 129,772 | PASS (first ever run) |
| 5205 | unreal_engine-01 | 8/8 | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 65,536 | PASS (first ever run) |
| 5206 | catia-07 | 6/6 (incl. 03, 06) | 0/0 | 0 | 0 | 0/0/0/0 | 8,005, 1,042,740 | PASS (Phase 1 path exercised) |
| 5207 | creo-04 | 13/13 | 0/0 | 0 | 0 | 0/0/0/0 | 9,112, 1,043,244 | PASS |
| 5208 | blender-01 | 7/7 (incl. 01) | 0/0 | 0 | 0 | 0/0/0/0 | 0, max entry 557,004 | PASS |
| (Q4 4630-4633, host campaign 5101-5104) | energy-04 | 6/6 each | 0/0 | 0 | 0 | 0/0/0/0 | - | PASS 8/8 (other lanes, same revision) |

Lane: a generalised copy of the Q4 round-6d Windows lane (`lane/` here: `spec-arm.py N REV LABEL WORKLOADS ITERATIONS [SECONDS]`,
`spec_session.py`, `campaign.py`, `summ.py`); the guest-agent, observer, broker launcher and Xid helpers are the unchanged Q4 host copies.

## Nine viewsets in one boot (MEASURED)

| run | sequence | result | first failure signature |
|---|---|---|---|
| 5209 | blender, catia, creo, energy, enscape, maya, medical, solidworks, unreal (the run-3440 order), 1 iteration | FAIL after 54 min: blender, catia, creo, energy-04 Completed; enscape 01-06 Completed; stopped by the lane's first-host-Xid rule during enscape 07 | host `Xid 31`, MMU fault, ENGINE CE0 HUBCLIENT_CE1, FAULT_PTE ACCESS_TYPE_VIRT_WRITE, on the host twin of a Windows USER passthrough ring (DmaCopy + ThreeD + Compute class set, own VAS); guest 153/4101 = 0 up to the last sample; no Budget/SCAN_BUDGET/NoCeObject/poison |

**Correction (2026-10-24, after run 5210; supersedes the paragraph below):** the disable/free/unmap cascade cited below is the
ACPI shutdown that the lane issues after the Xid, not process teardown (device time: fault at t~3259.8 s, cascade from t~3260 s,
guest RM unload at t=3268.9 s in run 5210). The fault occurs while subtest 07's process is running. The "unmap before
DISABLE_CHANNELS" reading is withdrawn.

| 5210 | same as 5209 (rerun, same binary) | FAIL at the same point: enscape 07, uptime 54 min 19 s (5209: 54 min 29 s) | same signature, same host channel id 0x6b (the per-process graphics channel every viewperf process receives), different VA |

Single-viewset enscape-01 (5201) passed subtest 07, so the fault needs the earlier workloads of the same boot (INFERRED; not isolated).

Context read from the qemu log (MEASURED order of log lines, INFERRED meaning; SUPERSEDED by the correction above): at that moment the guest was tearing down the
subtest's viewperf process; a 1,572-run unmap of that channel's address space was applied before the guest's
DISABLE_CHANNELS for the faulting twin. Whether the faulting VA lies in that unmap is not logged without `KF3_MAPLOG`.
Hypotheses and falsifiers are in the operator report; nothing was changed in product code.

## Diagnostic attempt with `KF3_MAPLOG=1` (run 5213, MEASURED; confounded by the flag)

Same nine-viewset order. blender, catia, creo, energy-04 Completed; during enscape 06 (UltraHQ) the guest logged nvlddmkm 153 x3 and
one RM unload/reload, with no host Xid and no relay refusal; the subtest made no progress for 19 min and the arm was stopped by its
own finish flag. The run never reached subtest 07, so the fault-VA history question stays open. MAPLOG adds a log write per doorbell
and per diff run (about 1 MB/s here), so this TDR is not attributed to the product (enscape 06 already needs about 1.4 s of GPU time
per frame on kayfabe without the flag). Harness-only changes for this run: extra-flag plumbing, tail-seeking byte-bounded log reads,
status-read retries (`lane/`).

## Status per viewset on candidate 6d (correctness)

| viewset | alone, fresh boot | inside one boot after blender/catia/creo/energy |
|---|---|---|
| blender-01, catia-07, creo-04, energy-04 | PASS full | Completed in 5209, 5210, 5213 |
| enscape-01 | PASS full (5201) | FAIL 2/2 at subtest 07 (host Xid 31, CE0 FAULT_PTE VIRT_WRITE, the subtest's graphics channel); TDR at 06 with MAPLOG |
| maya-07, medical-04, solidworks-08, unreal_engine-01 | PASS full (first ever kayfabe runs) | not reached (behind enscape) |

Open (not implemented): the faulting page's map/unmap history (needs a lighter VA diagnostic than MAPLOG), whether the earlier
workloads are needed (energy + enscape only), and whether the 8192 MiB store's VRAM pressure is needed (same sequence with a
larger store).

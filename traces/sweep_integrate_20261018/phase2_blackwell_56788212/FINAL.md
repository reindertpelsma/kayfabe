# GB202 Blackwell stage: bounded closeout

STATUS: LIVE, 2026-10-20. This lease is closed. Results below are MEASURED unless
explicitly marked INFERRED or CODE READ. No Blackwell family acceptance is claimed.

Own Vast instance **56788212**, RTX 5090 / host-derived GB202, ran from
2026-10-19 20:48:41 UTC until API-verified absence at **23:41:48 UTC** (2.885 h).
The 180-minute auto-destroy timer was armed before SSH and cancelled only after
absence. All cited evidence was exported and pushed before destruction. The final
host check found no QEMU, no connected forensic NBD, and healthy open 595.91.07.
The trusted host and its GPU were never used.

Accepted product source: `24906bf334665c9000b16b271005593daa3586c7`, descendant of
candidate-5 `f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`. Later campaign commits
change evidence and host health checks; product trees remain byte-identical.
QEMU SHA-256: `b33394dd23e515994a4478747a52311d05f1a1bffbb300eb4415f2a0e22220ef`.
The isolated codec arm is the explicitly separate, unmerged source below.

## Validation matrix

| Die | Guest / actual build | Source and configuration | Result | Evidence |
|---|---|---|---|---|
| GB202 | Native Linux 595.91.07 | Accepted source; stock open host driver | PASS: gates 9/9 + gate 10, USER 12/12, raw 30/30, CUDA 4/4 | `native_gates.txt`, `native_raw595.txt`, `native_cuda595.txt` |
| GB202 | Linux 580.159.04 | Accepted source; host 595.91.07; frozen raw timeout 180 s | PASS: raw 30/30, CUDA 4/4, headless graphics 5/5 steps and 10/10 hashes | `linux580_raw/`, `linux580_cuda/`, `linux580_gfx/` |
| GB202 | Linux 595.91.07 | Accepted source; host 595.91.07; frozen raw timeout 180 s | PASS: raw 30/30, CUDA 4/4, headless graphics 5/5 steps and 10/10 hashes; heartbeat remains PARTIAL | `linux595_raw/`, `linux595_cuda/`, `linux595_gfx/`, `B3_RESULT.md` |
| GB202 | Linux desktop 580.159.04, baseline | Accepted source, actual Cinnamon/Xorg and X11 SHM broker | FAIL: black 1920x1080 frame, one flip timeout, zero guest Xid | `B2_RESULT.md`, `b2_baseline.txt`, `b2_pixels.json` |
| GB202 | Linux desktop 580.159.04, codec arm | `45a4d91b56b6819fa4e67687d7f3f348647a325e`, unmerged | FAIL: identical black frame and one flip timeout; native gates pass | `b2_codec_candidate/`, draft PR 51 |
| GB202 | Windows LTSC / 580.88, actual 32.0.15.8088 / oem7.inf | Accepted source, exact P19 flags from `../MATRIX.md`, display/GOP on | FAIL initialization validation / Code and TDR UNVERIFIED: one enable, then SSH and QGA unavailable; full suite UNRUN | `b1_preparation/`, `b1_on/`, `b1_forensics/` |
| GB202 | Same Windows build, fresh sibling disk | Same source, P19, display off / GOP off / empty GOP ROM; QOM readback verified | FAIL initialization validation / Code and TDR UNVERIFIED: one enable, then SSH and QGA unavailable; final trace has zero GSP RPCs | `b1_off/` |
| GB202 | Linux desktop 595; Windows second build | No additional arm | UNRUN | No acceptance claim |

Each complete suite was measured once per guest build. Graphics 580 used a native
595 baseline; ten matching hashes do not establish same-driver native parity.
Native/raw fault arms deliberately produce Xid 31: all 15 host records and their
workload attribution are retained in `HOST_NEGATIVE_FAULTS.md`. The complete guest
raw serial captures contain zero guest Xid and zero flip timeouts. Do not turn
that narrower observation into a zero-host-Xid claim for the whole lease.

## B1: display comparison remains inconclusive

CODE READ (commit 007ba1eb0:41): quoted historical statement, evidence not revalidated by BW9. Both measured Windows arms started from fresh children of the same stopped,
disabled-NVIDIA parent. The parent hash was rechecked; actual driver 8088, GSP
registry settings and disabled state were verified before the single enable in
each arm. Original TPM/Secure Boot identity was retained. No recovery, re-enable,
timer change or third initialization was attempted.

CODE READ (commit 007ba1eb0:47): quoted historical statement, evidence not revalidated by BW9. ON's early 99-RPC snapshot was transient. Its final log contains 386 RPCs,
including CA70 at ordinal 233, CA7A at 271/273/275/277, SET_RMFREE_FLAGS at 278,
first Free at 279, and explicit function-47 unload at **385**, following
20800AFE at 384. The final log was acquired during cleanup after powerdown was
requested; the late RPC timestamps are unknown. Therefore 385 is a final-log
unload ordinal, **not a proven StartDevice abort ordinal**. Alignment against the
passing current Ada trace matched 306/386 calls; unmatched calls remain explicit.

OFF's actual QOM readback was display=false, GOP=false, GOP ROM empty, ABI
580.65.06. It produced **zero GSP RPCs through final shutdown**, with no display
allocations and no unload. This is not the same initialized path without display;
it cannot falsify or prove the thin-display-caps hypothesis. The initial launch
with contradictory nonempty-ROM/GOP-off arguments was refused and is NOTRUN,
preserved in `b1_off_refused/`. The extra approval question was unnecessary:
section 2 already authorized the valid display-off configuration, which an
existing positive test and unchanged guard support.
The owner subsequently explicitly approved omitting the GOP ROM on 2026-10-20,
after this arm and the lease teardown. The reply is recorded without a new run.

Read-only extraction of persisted ON event logs parsed 748 records and found no
positive TDR or Code 43 event in the ON window. Missing persisted events after an
unresponsive guest and QMP shutdown do not prove that no such event occurred.
No opaque Windows reply bodies, EVTX/XML, VM images or executables were exported;
only screened metadata, public fields and hashes are committed.

CODE READ (commit 007ba1eb0:72): quoted historical statement, evidence not revalidated by BW9. CODE READ / measured limitations: CBCB remains NOT_DERIVABLE; old exact-GB206
native data had 120-byte replies with 43 distinct values, and there is no native
GB202 Windows reference. AB2 sizes differ between native 580 and host 595. No
neighbor-die or cross-ABI value was borrowed. No new CMC or caps-word probe was
justified by this run. The optional CE branch `95e23df77` was neither merged nor
run because no actual Windows TDR was confirmed.

## B2: public ring-format defect fixed separately, desktop still red

Both desktop arms displayed the same single-color black image, SHA-256
`788ae0147bdf979a6575938ca2d7d4403788588f7be2010f03776c968fd1ab49`.
The baseline and candidate each had one flip timeout and zero guest Xid.
The candidate consumed display methods and sent/released broker frames, so those
counters alone do not establish correct frame contents or completion ordering.
No third desktop arm was run.

The separate codec topic uses public headers for the CA7B/CA7D/CA7E address
fields and 64 KiB method-ring bound. Failing-first tests, nine mutants with at
least two killing tests each, release tests, clippy delta and header provenance
are complete. Counts: kf-disp 284/0/5 -> 303/0/5; kf-qemu 407/0/0 -> 416/0/0.
No existing tests or assertions were removed. It did **not** fix this B2 failure.
Draft [PR 51](https://github.com/reindertpelsma/kayfabe/pull/51) remains unmerged.

## B3: concurrency passes; heartbeat bursts persist

The normal-console concurrency arm passed in 41 s, client 19.350 s. The full
raw suite's concurrency arm passed in 33 s, client 19.030 s (n=2, observed values,
no causal performance ratio). A quiet-console arm was unnecessary because the
normal-console case did not reproduce the timeout. Neither the frozen 180 s
grader nor the 500 ms heartbeat period was changed.

The first concurrency boot had 31 raw-heartbeat-zero startup warnings within
217.306 ms. All 30 later raw serial logs had zero heartbeat warnings. However,
successful CUDA 595 CUP3 and graphics 595 boots produced **nonzero-heartbeat**
warnings. CUP3's 304 post-capture warnings form two bursts: 144 over 81.426 ms
and 160 over 61.867 ms, separated by 5.609766 s; overlapping pre/post captures
must not be added. Graphics had 104 before / 214 after, also overlapping.
Linux 580 CUDA and graphics captures had zero heartbeat warnings.

CODE READ (commit 007ba1eb0:111): quoted historical statement, evidence not revalidated by BW9. H_CLOCK_VIEWS falsifier was stated before probing: native PTIMER and VF_TIME
agree within the existing 10 ms bound in all three samples. Result: **falsified**
the native mismatch lead, deltas 5696/5696/5728 ns, UID 65534, no capabilities.
This does not prove guest epoch/lifecycle handling. INFERRED next lead: an epoch
transition or stale published heartbeat can place adjusted heartbeat ahead of
current time. That mechanism is not proved or patched; protected reset/scrub/
relay paths and all timing constants remain unchanged.

## Remaining questions and next work

1. B1: where does the ON path first diverge before cleanup, and why does valid
   OFF lose connectivity before any GSP traffic? Code 43 and TDR need a direct
   guest observation; final-log unload alone is insufficient.
2. Display reference: compare the existing current Ada public-field projection
   against an initialized Blackwell display path. A headless native display
   subtree is not a substitute, and no exact GB202 native Windows trace exists.
3. B2: locate the first missed display event or incorrect rendered contents
   between the guest ring and broker; the header codec fix is not the cure.
4. B3: correlate heartbeat publication with guest epoch/lifecycle transitions
   without blocking the drainer or changing its 500 ms period.
   `B3_CODE_REVIEW.md` rejects one metadata ordering race and states a falsifier
   for the still-unproved mailbox value / epoch transition lead.
5. Ampere and Turing current-product regression remains UNRUN. GA104 Windows
   still requires a verified native/VFIO binding using unmodified windows-on-vast
   before any kayfabe Windows acceptance attempt. No extra lease is left active.

No new owner approval is required for these existing work items. Any dependency
on protected lifecycle, birth/groups/latejoin, relay, display pacing or scrub
code must be reported under the existing STOP rule before changing it.

## Cost and cleanup

Own invoiced-so-far at closeout: **3.598 USD** across 56765201, 56766467 and
56788212; GB202 contributes 2.190 USD. This is not asserted final settlement.
Combined elapsed-runtime estimate: 3.683126 USD; conservative budget accounting
with three separate 1 USD delayed-billing reserves: 6.683126 USD, not actual
spend. Owner cap remains 25 USD with stop renting/report at 20 USD.
See `../phase2_cost_closeout_20261020.json`, `../phase2_owned_instances.json` and
`../../die_driver_sweep_20261015/COST.md`.

All three own IDs are API-absent. Own Rust target directories were deleted,
forensic NBD disconnected, no own VM or rental workload remains, and no trusted
host lock was taken. This closes the bounded Blackwell lease, with red results
and unverified cells retained.

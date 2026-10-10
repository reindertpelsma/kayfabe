# V3 — the HELD-hole (micro-reservation geometry): fix, evidence, handoff

**STATUS: LIVE, 2026-10-10 (handoff).** Fix CODE + MODEL-TESTED + HARDWARE-VERIFIED on the Windows
production profile (runs 381, 382, 384, 385 below); gates 9/9, the Linux fast suite on the final
revision and the broker lane are listed in "Still to run" with what was run. Parent text:
`V3_BATCHED_MAP.md` (the correction block at its top carries the same root cause).

## 1. The defect (measured, then inferred; kept apart)

*Symptom, measured:* Windows run 289 (integration + D1-D3 code, `daded67e`): 14 leaves logged
`HELD BY HOST`, 4 NEW host Xid 31 `FAULT_PTE` at exactly those VAs (`0x1499c000` GR0_PBDMA0/ESC ch 0x3c,
`0x14589000` CE0/CE1, `0x14993000` GRAPHICS FE, `0x14995000` CE3_PBDMA0/ESC). The eff1b692 line (runs 277-288,
290, 291) had 0 HELD and 0 new Xid; runs 272-276 and 289 (all D1-D3 code, micro reservations default ON) had HELD.

*Root cause, measured:* host RM does not hold the range a micro reservation asks for. It aligns the START
down and the SIZE up to the page size it picks for the request (2 MiB when the request is >= 2 MiB, else the
64 KiB big page): host dmesg of run 289 `virtmemAllocResources: VA Space alloc failed! Status Code: 0x51 Size:
0x20000 RangeLo: 0x144d0000` for a request at `0x144d5000`; `Assertion failed: vaHi <= pMemBlock->end @
gpu_vaspace.c:2022` + `dmaAllocMapping_GM107: can't update VA space for mapping @vaddr=0x14495000` (the nine
`batch fallback ... Map(Other(31))` lines); source `gvaspaceApplyDefaultAlignment` (`gpu_vaspace.c:1645`),
`_memmgrPickDefaultGpuPageSize` (`mem_mgr.c:1743`, ogkm 595.84). Direct hardware probe (this branch's
`kf-micro-reserve-probe reserve`): after `reserve_va(0x4030000, 8 pages)` a one-page NV01 map at `0x4038000` is
refused `VA_ALREADY_MAPPED`; a page below an unaligned reservation start is refused; an aligned 64 KiB
reservation holds exactly 64 KiB.

*Mechanism, inferred from the above (not separately instrumented):* the ledger (`BatchedVas::micro`) recorded the
requested range, so the pad was occupied on the host but invisible to `segments()`; the guest's later 4 KiB
rows in the pad got `VA_ALREADY_MAPPED` -> `HeldByHost` -> acknowledged HELD, never mapped -> engine fault.
Fits every datum: all HELD leaves and Xid VAs are tails of 64 KiB units (`0x1499c000+0x4000`,
`0x15bac000+0x4000`, `0x14588000+0x8000`). Gate 10 passed on the broken tree because it only used the 8 pages
it asked for.

## 2. The fix and where it lives

| commit | what | separable |
|---|---|---|
| `7581c9b7` (held-hole line `9e3fbbf0`) | `sim::SimRm` rounds reservations like the host (`reserve_rounds`, default ON), classifies fixed maps refused by a NON-foreign occupant (`self_held`), `sim::check` fails `SILENT ABSENCE`. 10 model tests fail on the old code: the GPU-free repro | tests only |
| `dfa285dc` (`40396c70`) | `batch.rs`: `reserve_exact/reserve_cores/reserve_hull/batch_core`; `place()` reserves the run's aligned hull when nothing of ours is in the pads (ledger records RM's real block; later pad rows map THROUGH it) else the aligned core, head/tail per run at 4 KiB; `leaf_segments` reserves aligned cores + grains the remnants; leaves < 64 KiB are 4 KiB grain; 2 regression tests + fixture updates | the fix |
| `6862a5db` (`02b984e5`) | probe measures the rounding and gates "an aligned 64 KiB reservation holds exactly 64 KiB" | gate |
| `4db48053` (`83580cf9`) | `BatchedVas::held_ours` + `HELD-BY-OURSELVES` log: a HeldByHost for a VA our ledger says is ours; doc correction | counter |
| combined `183e1b8f` | `claude/windows-combined-20261010` = `origin/claude/display-latch-contract-20261010` @ `c028265e` (display latch contract, late joiner, interlock, video channels) merged with the four commits above | integration candidate |

**The combined line is the integration candidate** (coordinator, 2026-10-10): `COMBINED_LINE.txt` on the
host names its head. Do NOT merge `tdr-opus-base-20261010` into it (a second diverged port of the display fix).
The kf-mem commits are separable (cherry-pick `7581c9b7 dfa285dc 6862a5db 4db48053` onto any line that has D1-D3).

Tests: `cargo test --release -p kf-mem -p kf-qemu -p kf-disp -p kf-harness -p kf-rm` all green at `183e1b8f`
(kf-mem 171 lib tests, +2 regressions `a_batch_leaves_no_unrecorded_pad_the_guest_cannot_map`,
`a_foreign_buffer_in_the_pad_makes_the_reservation_impossible_not_the_leaf_silent`); hostile fuzz
`KF_FUZZ_SEEDS=2000` (and `KF_FUZZ_BASE=5000`) clean; property tests at 1500 seeds (5x) clean; CI python
tests 38 OK. The pre-fix tests that asserted an exact-block reservation were corrected, not weakened
(`eight_scattered` keeps its vidmem neighbour outside the 64 KiB unit; the leaf-bound test expects the aligned
core through the reservation + the 4 KiB tail at grain).

## 3. Hardware results (host 172.22.1.20, RTX 4070 595.91.07, Windows 11 production profile, zero flags)

Runner: `tdrhunt/tdr-run15.sh N` (scripted sign-in, Edge, YouTube Shorts consent + play, 900 s hold).

| run | binary | HELD BY HOST | new host Xid 31 | micro refusals | batch fallbacks | TDR boot / signin / edge / shorts / end |
|---|---|---|---|---|---|---|
| 289 | `daded67e` (unfixed D1-D3) | 14 | 4 | 37 | 9 | 2 / 3 / 3 / 3 / 3 |
| 291 | `a82e5c09` (eff1b692 line) | 0 | 0 | 0 | 0 | 0 / 0 / 0 / 0 / 2 |
| 381 | `02b984e5` (integration + fix, no display fixes) | 0 | 0 | 2 | 0 | 1 / 3 / 3 / 8 / 8 |
| 382 | `50c13332` (+ tdr-opus-base merge; stopped at hold 260 s by mistake) | 0 | 0 | 0 | 0 | 0 / 0 / 0 / 1 / 2 (partial) |
| 384 | `4db48053` (combined, before c028265e) | 0 | 0 | 3 | 0 | 0 / 0 / 0 / 1 / 4 (resets at hold 0/261/563/844 s) |
| 385 | `183e1b8f` (combined) | 0 | 0 (24 = 24) | 2 | 0 | 0 / 0 / 3 / 3 / 3 (no reset during the 900 s hold; `PLAYBACK frames differ: 0` = the two Shorts screenshots are identical: playback not demonstrated) |

The TDR counts of 381 differ from 384 because 381 lacks the display/late-joiner fixes (a different defect);
the periodic resets of 384 (~290 s) are the TDR hunt's, not this fix's (HELD 0, Xid 0). Linux fast suite
(`KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh heldhole 180`, host `/workspace/bench/heldhole_suite.out`):
**30/30 PASS, 0 FAIL, 0 CRASH** — but MIXED revisions (the first 16 arms ran on `4db48053`, the other 14 on `183e1b8f`
because the host worktree was advanced mid-suite). The clean single-revision rerun
(`heldhole3`, `rev=183e1b8f`, outer lock `/tmp/kayfabe-fastguest.lock`, started 20:31) is **30/30 PASS, 0 FAIL, 0 CRASH**
(`/workspace/bench/heldhole3_suite.out`). NOTE: a first rerun (`heldhole2`) CRASHED every
arm with `realize refused: store of 8192 MiB refused: NoMemory` because another agent's Windows VM (run 400) held the
GPU while the arms ran: environmental, not a regression — never start arms while a QEMU holds the card.

## 4. Runbook (what I did; repeat it)

1. Build: `cargo build --release -p kf-qemu` locally (`CARGO_TARGET_DIR=/root/kf-held-target`; the box disk is
   nearly full: no debug builds, `cargo test --release`). Ship `libkf_qemu.a` to the host (`/root/held-tgt/release/`),
   host worktree `/var/lib/kf-windows-20261005/kayfabe-heldhole` (sparse: `scripts/bench`, `scripts/fastguest`,
   `qemu/hw/misc/kf3`), `git checkout --detach <sha>`, then `PATH=/root/held-shim:$PATH CARGO_TARGET_DIR=/root/held-tgt
   bash scripts/bench/build_kf3.sh $W/win-qemu/qemu-10.2.4 $W/win-qemu/qemu-build-kf3` (`/root/held-shim/cargo` is a
   no-op: the host has no cargo). It installs `win-qemu/kf3-bins/<sha>`; the runner reads `$W/kf3-bins/<sha>`, so
   `cp -a` it there (and into `/workspace/bench/kf3-bins/` for the fast suite). `W=/var/lib/kf-windows-20261005`.
2. Run (through the shared lock, never holding it idle):
   `cd $W/tdrhunt; nohup flock -w 28800 -o /tmp/kayfabe-fastguest.lock env KF_GUEST_PW=kfsign7 KF3_REV_BIN=<sha>
   HOLD_SECS=900 RUN_WIN_FLAGS= bash tdr-run15.sh <N> > /root/held-run<N>.out 2>&1 &` (pick an unused N; a refused run
   still burns its N's `winprod/run<N>`; do NOT kill a runner mid-hold: its cleanup takes minutes and blocks the
   queue; if you must, `kill -TERM` the bash script and wait for `END type=DMA-FQ`).
3. Count: `L=$W/boundary-kayfabe-<N>/qemu.log`: `grep -a -c "HELD BY HOST" $L`, `HELD-BY-OURSELVES`, `batch fallback`,
   `micro reservation.*refused`; Xid: `xid_now` vs `before` in `$W/winprod/run<N>/evidence.log` and `dmesg | grep Xid`;
   TDR per phase: `grep -a PHASE $W/winprod/run<N>/winprod.log`. Playback: the `PLAYBACK frames differ:` line.
4. Host dmesg is the authority for VA problems: `virtmemAllocResources: VA Space alloc failed` and `dmaAllocMapping_GM107`
   lines name the rounded range RM tried.
5. Leave the host clean: IOMMU group 11 `DMA-FQ` (`cat /sys/kernel/iommu_groups/11/type`), no qemu, no
   `/tmp/kf-stop-winprod`.
6. Linux: `KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh <tag> 180` from the host worktree (the binary is
   `/workspace/bench/kf3-bins/<rev of the checkout>`; each arm takes the lock itself; ~4 s per passing arm).

## 5. Still to run / open items

- **Gates 9/9 and gate 10 on the final revision:** not run in this session (the gate binaries need a full release
  build of the harness crates and the box root disk was ~2 GB free). Gate 10 (the probe) PASSED on `02b984e5`
  (same kf-mem and probe code; measured lines above). Run `scripts/bench/v3_gates.sh` before integrating.
- **Linux fast suite 30/30:** DONE at `183e1b8f` (single revision, see the table); not re-run at `3aea0a79`+ (arithmetic-only change).
- **Broker lane / desktop retest** (`broker_lane.sh`, `interactive.sh`; owner memory: after interrupt/GSP/drainer/display
  changes): not run.
- **OWNER_RULINGS §AD (12 ms invalidate latency bound):** the fix adds no sleeps and no blocking; the refresh's host
  calls are unchanged or fewer (a refused/impossible reservation now goes straight to the 4 KiB grain). Not
  re-measured against the 12 ms bound; the `INVAL-SLOW` diagnostics live on `tdr-opus-base` (not merged here).
- **Reviewer items:** `3aea0a79` (independent review, on top of `183e1b8f`) cleared 13 `arithmetic_side_effects` in
  `batch.rs` (same results) and corrected the docs: `held_ours` is a test-read atomic + stderr line, not an `Applied`
  field, and it would NOT have fired in run 289 (the pad was unrecorded): the real hardware gate is the plain `HELD`
  count = 0 with 0 new Xid. It also marks the 2 MiB rounding as an unmeasured assumption; note the run-289 dmesg does
  show it (`Size: 0x800000 RangeLo: 0xa00000 ... pageSzLockMask: 0x211000` for a request at `0xa70000+0x7e9000`), so
  the 2 MiB start-down/size-up alignment is measured for reservations >= 2 MiB, though never exercised by a probe
  arm or by the sim beyond 96 pages. Any further reviewer item: append here. The hardware runs above are at
  `183e1b8f`; the head after the review commit is `3aea0a79` (tests green: kf-mem, kf-qemu), not re-run on hardware.
- **Residual design risks:** the 64 KiB unit is the Pascal+ big page (Maxwell 128 KiB not handled); a neighbour of ours
  or a host buffer inside a 64 KiB unit makes the reservation impossible (`NoMemory`) and the rows go per run
  (correct, slower; the 3 refusals of run 384 are this); a genuinely host-owned occupant is still `HELD`
  by design (the walker keeps the HELD record after the host buffer is later freed: a hole until the guest changes
  the leaf — not addressed); head/tail grains of an `over`-bound segment are budgeted per refresh
  (`REFRESH_*_BUDGET`) like other grains.

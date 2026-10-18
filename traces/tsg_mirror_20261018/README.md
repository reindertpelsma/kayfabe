# One host TSG per guest TSG, one real host ctxshare per guest ctxshare: implementation and gates (2026-10-18/19)

**STATUS: RESULT, 2026-10-19.** Branch `claude/tsg-mirror-20261018` (from `integration/prod-candidate-4-20261017` 0452613b; candidate-5 `f8f9d1a5` merged before the
hardware phase, so the binary is candidate-5 + this change). Not merged. Design: `docs/design/V3_TSG_SUBCONTEXT_FEASIBILITY_20261018.md` section 10.
Labels: MEASURED = a run named here. CODE READ = source. INFERRED = reasoning. Binary: host `/data/claude-tsgmirror-20261018/kf3-bins/0d7ba743/qemu-system-x86_64`,
md5 `48ab1b70c2cf2c26d2c1287bf7311a9a`, built from commit `0d7ba74389` (code-identical to the branch head; later commits are docs/traces only).
Kill switch: `KF3_TSG_MIRROR=0` (measurement class, default ON, read once).

## Verdict
* **The latency falsifier FAILED to hold the old hypothesis, i.e. the hypothesis is CONFIRMED: the tiny-packet GPU leg was cross-host-TSG switching.** MEASURED, Windows 11 D3D12
  round trip (guest `sl_probe.ps1`, 3 x 3000 submits per queue type, medians): DIRECT 840/849/851 us (base) -> 268/262/279 us (mirror); COMPUTE 1486/1558/1483 -> 229/239/233 us;
  COPY unchanged (one TSG anyway) 193-198 us. Steel Nomad (3DMark Demo DX12, same disk overlay pattern): base 2479 2432 2478; new binary with mirror OFF 2469 2493 2481;
  **mirror ON 2876 2881 2996 2908 2929 2943 (+18 %)**; VFIO reference from the earlier lane 3795-3962 (not re-measured, so the remaining gap is not closed).
* 15-minute-plus D3D12 hold on the final binary (boot 23:13 UTC, six consecutive Steel Nomad runs, 13 min of load, then three latency runs): System 4101 = 0, host Xid delta 0, 0 device resets after
  boot, 0 refused acts of 5 873, `mirror=[... groups=156 ctx_created=213 ctx_freed=193 mode_refused=0 flag_refused=0 ctx_unknown=0 cap_refused=0 latejoin_skipped=74 unwind_failed=0]`.
  The 74 skipped late-joiner schedules are joiners of standing explicit groups (ASYNC ctxshares, host VEID 0x3f, runqueue bit set, exactly the D3D12 shape): the joiners fetched without
  their own schedule (G5d: the rule is dead on the new path; `latejoin.rs` NOT deleted).
* **One hardware defect found and fixed (retry 1)**: the first build (`df944612`) put Windows into repeated driver unload/reset about 35 s and 63 s after boot (runs 3010, 3012; the
  kill-switch arm 3013 and base 3011 were fine). Cause (MEASURED in `qemu.log`): `act deferred API context REFUSED (0x40): ... evict: Other(19314)`, 19314 = `HOST_ABI_REFUSED`: the
  channel-level `GPFIFO_SCHEDULE` (`0xa06f0103`) that explicit groups use for evict/stop had no row in `HOST_CONTROLS`, so the 595.91 host refused it. Fixed with a listed row
  (RED commit then GREEN commit, 2 unit tests, mutation M13 kills both). Not found by any GPU-free test because the carry gate needs the real host driver version; both tests now pin it.
  Run 3010 also logged one host Xid 31 (CE2_PBDMA0, FAULT_PTE read) on that defective build, not reproduced on the fixed one (so the host Xid total moved 15 -> 16 once, run 3010).

## Gate table (rev = binary revision; n = runs; every Windows arm on a fresh qcow2 overlay of the owner's disk, owner's disk untouched)
| Gate | Result | Rev | n / spread | Evidence |
|---|---|---|---|---|
| G1 `v3_gates.sh` 9/9 + gate10 | **PASS** 9/9, births 12/12 USER | df944612 harness (kf-host unchanged since) | 1 | `evidence/g1_log.txt` |
| G1 `fast_suite.sh` 30/30 (Linux raw client) | **PASS new 30/30, base 30/30**, Xid 15 -> 15 | df944612 / 0452613b | 1 each | `evidence/g1_suite_totals.txt`. Only LEGACY TSGs are born there (114 status lines `mirror=[... groups=0 ...]`, 10+ `BIRTH mode=LEGACY`): this gate shows legacy is unchanged, it does NOT exercise the explicit path |
| G2 CUDA ladder (cup2/cup3/cup8, streams, llm_g1) | **UNRUN** | - | - | no provisioned Linux fat guest (gcc + libcuda qcow2, `gfx_lane.receipt`) on this host; those lanes were provisioned on vast boxes. CUDA's TSG shape is therefore still unmeasured on hardware |
| G3 Vulkan/graphics (`gfx_suite.sh`, old Xid-69 shape) | **UNRUN** | - | - | same reason |
| G4 video lane | **UNRUN** | - | - | same reason |
| G5 Windows boot, sign-in, desktop, D3D11/D3D12 | **PASS** on 0d7ba743; FAIL on df944612 (defect above) | 0d7ba743 | 1 boot | run 3014 |
| G5a latency probe | **PASS (falsifier not met: leg shrank)** | 0d7ba743 vs 0452613b | 3 vs 3 | `evidence/lat_*.txt` |
| G5b Steel Nomad | **PASS, +18 %** | 0d7ba743 | 6 vs 3 base, 3 kill-switch | `evidence/sn_series_mirror_on_run3014.log` |
| G5c 15-min hold | **PASS** (0 TDR, 0 Xid, 0 resets) | 0d7ba743 | 1 | counters above |
| G5d joiners without the late schedule | **PASS** (74 joiners, all fetched) | 0d7ba743 | 1 | `evidence/run3014_tsg_mirror_lines_head.txt` |

### Latency detail (us, medians of 3000 submits; `evidence/lat_*.txt`)
| arm | COPY | DIRECT | COMPUTE |
|---|---|---|---|
| base 0452613b (3011 / 3015 x2) | 280 (first run of the boot) / 193 / 194 | 840 / 849 / 851 | 1486 / 1558 / 1483 |
| candidate-5 binary, mirror OFF (3013) | 199 | 936 | 1487 |
| **mirror ON 0d7ba743 (3014) x3** | 197 / 193 / 198 | **268 / 262 / 279** | **229 / 239 / 233** |
The previous lane measured 231 / 834 / 1485 on the production binary: reproduced. The last-marker-to-first-GPU-interrupt hop (5.6 / 390 / 1213 us) was NOT re-traced (no bpftrace profile run); the
round-trip drop of 570-1250 us is the same size as that hop, which makes it the cause, but the hop itself is INFERRED, not re-measured.

## What the guests actually send (first hardware record; the study said no trace had it)
`CTXSHARE-FACTS` / `TSG-MIRROR BIRTH|CTX` in `evidence/run3014_tsg_mirror_lines_head.txt`. Windows: ctxshare `0xff0e0200` flags 0x0 = SYNC, `declared_subctxId` 0, `hVASpace` 0 (so the host ctxshare
names the channel's twin space, never 0); joiner ctxshares `0xff0e0201..` host kind ASYNC, host VEID 0x3f, channel flags `0x..30` (privileged + runqueue 1), host rq1 = true. Most Windows
clients (DWM, apps) are single-ctxshare EXPLICIT TSGs (SYNC, VEID 0); the kernel driver / video channels are LEGACY. The guest-side VEID is not visible to kayfabe (the alloc reply is our echo).

## Tests (GPU-free)
`cargo test --release -p kf-abi -p kf-host -p kf-chan -p kf-qemu -p kf-rm`: base candidate-4 **2241 passed / 0 failed** -> branch (candidate-5 merged) **2349 / 0**. This change adds kf-abi +3,
kf-host +6, kf-rm +3, kf-qemu +32 (`tsgmirror`) and extends the structural birth guard (`channel_budget_lazy`); the rest of the kf-qemu delta (291 -> 387) is candidate-5. Process: tests first, each
area committed RED (`4a5e5a5af`, and the `0xa06f0103` row) then GREEN. clippy on the new code is clean (pre-existing warnings elsewhere untouched); rustfmt applied to the new/clean files only
(`chan.rs`, `userrelay.rs`, `chanlink.rs` were not rustfmt-clean at base, left alone).
Mutation table (13 mutations, each fails >= 2 tests): `evidence/mutation_table.txt`.

## Safety shape (hostile guest)
Host actions are authored: ctxshare kind is SYNC/ASYNC decoded from validated flags (the guest never names a VEID), host channel flags = belt + runqueue bit only (every other guest bit is
consumed or refused by name: bits 1:0, 2, 25, 26, thread 3), hVASpace never 0, 64-subcontext cap refused by name, mode/engine/space/parent contradictions refused 0x1f before any host call, unknown ctxshare
0x57, host failure unwinds exactly what the birth made. No flag is required to boot.

## Still unknown / must be verified before this may become the default launcher
1. **Linux CUDA** (explicit single-ctxshare, probably ASYNC VEID 0x3f, 8 channels, real kernels, the SKEDCHECK05 non-default-stream case): G2 unrun. 2. **Linux Vulkan** async compute + graphics (the old Xid 69 shape) and the
   runqueue-bit theory: G3 unrun. 3. **Video lane** (legacy, expected unchanged): G4 unrun. 4. UVM lane, `broker_lane.sh` / `interactive.sh` (the act path changed): unrun. 5. Windows: only D3D11/D3D12 Steel Nomad + the
   probe were exercised; D3D12 games / Edge / GPU-P compute, driver unload/reload and S3 resume on the new shape are unrun. 6. Hop 4 (marker -> first GPU interrupt) not re-traced. 7. The VFIO gap (3900 vs 2900) remains.
   8. Candidate-5 is in the binary: the mirror-OFF arm (2469-2493, 936 us DIRECT) shows candidate-5 alone does not explain the gain, but candidate-5 alone is not separately cleared for Windows by this lane.
   9. `latejoin.rs` deletion needs probe 60-66/90 re-run on the host driver in use plus a Windows hold without the legacy path (the new path never calls it for joiners, 74 of 74).

## Final host state
GPU nvidia 595.91.07 healthy; flock released; owner VM restarted as **run 3009 on 0452613b** (`OWNER_SESSION_READY 01:40:35`); host Xid total 15 -> 16 (one, run 3010, defective build). Written only under
`/data/claude-tsgmirror-20261018/` (plus the kfagent diag queue under `/data/kfagent`, the existing channel the guest polls). Launcher copy `win-launch.sh` with `WIN_DIR` + overlay disks; `/data/claude-manual-owner-perf4.sh` untouched.

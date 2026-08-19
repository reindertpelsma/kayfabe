> ### ⊘⊘ SUPERSEDED IN PART BY w330 (2026-08-19) — READ `w330_the_bench_rebuild_and_three_flags.md` FIRST
> **§2 item 1 ("FLIP TWO DEFAULTS") is DONE as a measurement and its framing was INCOMPLETE.**
> - The bench is rebuilt on a **fresh GA106** (580.159.04, deliberately downgraded to match every
>   recorded green boot). cup3 `=43`, cup8 `bad=0 maxerr=0` @2048².
> - ⊘ **There are THREE default-off flags, not two.** `KAYFABE_JOIN_RELEASE=supersede` is the
>   third, and it is a **correctness** flag: the shipped default (`arm=on`, leg 1) fires **zero
>   times** on this workload and is behaviourally identical to `off`. Measured 2/2 clean vs 2/2
>   host-Xid `FAULT_PDE`.
> - ⊘ **The two perf flags act on DIFFERENT STATISTICS and neither is sufficient.** Gate: median
>   8.5× / p90 19.4×, **max unchanged**. Coalescer: **max 12.7×**, median 1.6× **worse**. Graded on
>   one statistic, one of them reads as a regression.
> - ⊘ **§4's `0x110094` line of thought is REFUTED** — that register appears ONCE here. The hot
>   register is `0xbb0090` at **100.0 % share, 12.28 s, 25.4 ms mean** — our own doorbell.
> - ⚠ **The nesting caveat is now load-bearing.** ≤2.2 ms/launch was 2.6 % of an 85 ms trap and is
>   **up to 55 % of a 4 ms one**. Bare metal matters more *because* w318 worked.
> ⇒ **The next-step ordering below still holds from item 2 onward.**

# RESUME HERE — 2026-08-15

**STATUS: LIVE.** Written at master `68bd47f9` immediately after w329 merged, with **nothing in
flight**: both benches idle, no ungraded boot, no uncommitted work. ⊘ Supersedes
`../../../nvidia-gpu-passthrough/docs/design/RESUME_HERE_2026_08_12.md`, which has a redirect at
its head.

⚠ **Read the merge commits, not a summary of them.** `git log --merges --since=2026-08-14` in this
repo is 29 merges, each carrying its rung's findings, refutations and traps in full. **This file is
the ORDERING, not the record** — the record is already durable in git and in the memory index.

---

## 1. What is true now

- **cup3 `CUP3_VAL=43`** — 6/6 across today's rungs (not 1 boot; that caveat is retired).
- **cup8 `bad=0 maxerr=0`** at 2048², and at **N=3072 (36 MiB operands)** over 12 iterations.
- **`R33 arm 1`** fires — ⚠ but was **vacuous for three of today's four fixes**; check liveness per
  rung, never count it by default.
- **The 32 MiB "cliff" is closed** and was never about size: `KAYFABE_BENCH_BW=28,31` passes 3/3,
  `already joined` refusals **32 → 0**.
- **The BQL disposal is bounded** (worst hold 3.70 s → 54.8 ms).
- **The doorbell trap is 20.9× faster** (85.2 → 4.08 ms), correctness identical on all arms.

### What is NOT true yet
- **Publication still runs inline on the vCPU thread.** `pubqueue` + `trapwitness` are **built and
  unwired** — blocked on a real refactor: `Regs` is a `Box` behind a QOM raw pointer, so no worker
  can hold the thing that publishes.
- **The invalidate trigger is decoded but NOT armed** (arming turns a bug in our path into a guest
  **hang**, not a contained fault).
- **`supersede` is armable but not default** — its choice of winner between two live aliases is
  unproven, and `SUPERSEDE CAPPED` = 266/588 means **the cap is load-bearing**.
- **Second architecture unbooted**; `Ad10xArch::mmu()` still delegates to `MockArch`'s invented
  GMMU format.

---

## 2. ★★★ THE NEXT-STEP ORDERING — this is why this file exists

**In order. The first item is the cheapest thing on the board with the most evidence behind it.**

1. **⚡ FLIP TWO DEFAULTS.** `w318`'s dirty gate and `w321`'s coalescer **both ship default-off**.
   Master sits at **1.01× of its 3000 ms budget** without them — i.e. **98.7 % consumed**, which is
   the truncation that produces the `FAULT_PDE` intermittent. With the coalescer: **`complete=true`
   14/14, margin 2.34×–47.62×**.
   ⇒ **Already measured, already merged, one flag each.** Needs boots to confirm, not design.
   ⚠ Grade it on `complete=` / `pinned==asked`, **never on a green run** — anything that makes the
   doorbell faster makes the truncation rarer without fixing it.

2. **Rebuild the reclamation model around ALIASES, not unmaps.** `w329` measured that **CUDA's
   suballocator does not unmap on `cuMemFree`** — the guest VAS ends as one 140 MiB run holding both
   buffers, and the assumed release trigger fires **zero times**. The working mechanism is a
   **takeover of a join whose frame the guest re-pointed** (22/boot). Everything written about
   "release on the guest's unmap" needs re-reading against that.

3. **The `Regs` ownership refactor**, which unblocks the off-BQL execution site — and only then
   arm the invalidate trigger, because arming without the worker buys 1.6× while the dominant site
   is untouched.

4. **Operands.** Ours run at **2.51 GB/s** vs sysmem's 12.33 and VRAM's 313.5.
   - **Fix 1 — huge-page-back the leaf memfd: ~4.9×, one allocation site, no address moves.** Gate
     it on the `pde_info` measurement (already-built ioctl). **Do this one first.**
   - **Fix 2 — host VRAM operands: ~14.8×, hard.** The guest's view survives by installing the
     isolate's **BAR1 mapping** of the VRAM object as the `FbJoined` backing. Costs a BAR1 window
     manager (256 MiB BAR vs 12 GiB advertised FB), an unmeasured BAR1-read penalty on D2H, VRAM
     capacity, and the self-concealing `ShadowsGuestMemory`/#12 correctness class.

⇒ **Two separable performance problems, and they bind at different kernel sizes.** At N=128 submit
is **78 %** of the launch (LLM-shaped: many small kernels); at N=2048 it is **0.5 %** and the
compute is 22–81× off native. **Do not conflate them.**

---

## 3. Branch triage — measured at master `68bd47f9`

★ **All of today's thirteen rungs (w317–w329) are MERGED.** 29 merges today. Nothing below holds
current work.

| classification | branches |
|---|---|
| **MERGE-ME (superseded content already on master; verify then delete)** | `w310-pin-release` (+10), `w311-matmul-bench` (+3) — both merged via later rungs |
| **LIVE — needs a decision** | `fb-join` (+9, 08-11), `w295-test-suite-and-hw-tier` (+7), `userd-guest` (+4) |
| **UNKNOWN — cannot classify without a boot** | `gsp-plane-census` (+3), `status-divergence` (+2), `cpu-view` (+2), `w295-second-arch-boot` (+1), `schedule-0xa06f0103` (+2), `vchid-userd-oracle` (+1) |
| **STALE (`bx/*`, 07-30 → 08-01, pre-campaign)** | `task156` +6, `reachability` +5, `gspalloc` +5, `sweep-deviceinfo` +2, `qom-shim` +2, and 5 × +1 |
| **DOC-ONLY** | `w293-road-to-v1-doc`, `w272-the-announcement`, `w296-rescue-uncommitted`, `w259-orphan-triage`, `w295-class-level-consistency`, `sandbox-pivot-port` |

⊘ **Nothing here was merged, deleted or fixed as part of this park** — classification only.

---

## 4. Parked, with evidence, so they are not rediscovered

- **`vcpu_skipped=2` on a DISARMED control** (w326) — two concurrent or re-entrant entries into the
  retired drain **with no worker in existence**. Unattributed; `try_lock` refuses both where
  `lock()` would have deadlocked. **A master defect, found by a rung that was not looking for it.**
- **`INLINE-SAFE` clauses (a) and (b) are still prose.** `trapwitness` makes (c) structural but
  `at_a_host_verb` still mints a **counted** exception on a trap thread, so the gate cannot panic.
  Finish line = **mint census empty** (4 sites / 3 files today); two rows are annotated
  *"expect to survive"*.
- **`0x83de030c` (`DEBUG_READ_ALL_SM_ERROR_STATES`)** reaches the unserviced ledger in **exactly the
  11 failing boots and none of the 10 passing ones** — libcuda asking *which fault killed my
  kernel*. Refusing it does not cause the failure; **it blanks the guest's diagnosis.**
- **Mode 1 inherits a hazard Mode 2 is structurally immune to** — `DEFER_TLB_INVALIDATION` is
  cross-tenant, plus an aliasing bug NVIDIA's own source labels *"This is a bug"*. Mode 2 **authors**
  its host flags as literals; **Mode 1 forwards verbatim.** Accumulate the Mode-1 deltas as they are
  found.
- **The stable red set is 7 tests on 4 targets** at master (w329 re-verified; earlier rungs said 6/3
  and 3 targets — **the count moved and was mis-stated twice**).

---

## 5. Traps that cost real time on 2026-08-14/15

- **`build_qom_shim.sh` refuses an archive >30 min old** on an unchanged tree ⇒ **any sweep longer
  than 30 minutes fails its later boots as a BUILD refusal** that looks nothing like a workload
  problem. `touch` a crate source between arms.
- **A sequential arm sweep confounds arm with session time.** An arm looked like a regression;
  re-running **the control last** exonerated it. **Interleave.**
- **Never run two detached batches under the same tag** (done twice); **never `scp` into the clone**;
  **"byte-identical to master" is a claim about a moving ref** — pin your base.
- **`w291_r33.sh` hardcodes a clone path absent on `vh2`** — exits 90 with **no log**. Use
  `w317_r33_repeat.sh`.
- **Do not run `cargo fmt`** — the toolchain pin reformats 62 files of untouched master.
- **A stale `Cargo.lock` killed 5 arms in 43 s while the batch reported `rc=0`.**

---

## 6. ⊘ How to read anything written before today

Three campaign-wide corrections landed on 2026-08-14/15 and invalidate earlier phrasings:

1. **Four different things wear the word "drain"** — publication census/join (**cumulative** over 229
   traps), the guest-RAM **pin** drain (**one trap**, the worst), retired/disposal (40–56 ms), and the
   doorbell page-table pass. **`budget_hit=true` is the disposal's and is true on EVERY boot.**
   ⇒ **Name which, every time.** This misattribution propagated through three rungs.
2. **A measured zero is only as good as the transport list it ranged over** — *"the guest emits zero
   TLB invalidates"* was carried by three campaigns and is **false** (377 per boot, as BAR0 writes,
   sitting in every log as `UNCLAIMED-CENSUS`).
3. **`n = 1` is not a grade** — a single-boot `43` is wrong **1 time in 5** on these boxes.

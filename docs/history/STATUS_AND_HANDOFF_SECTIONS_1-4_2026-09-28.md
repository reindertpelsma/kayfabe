# Status and handoff, history part 4: sections 1 to 4 (master state, branches, owner decisions, investigations) as of 2026-09-30

**STATUS: SUPERSEDED, 2026-11-07** by [`../STATUS_AND_HANDOFF.md`](../STATUS_AND_HANDOFF.md) (current resume point) and [`../STATUS_DETAIL.md`](../STATUS_DETAIL.md). Sections 1 to 4 of `docs/STATUS_AND_HANDOFF.md` (master and its merge bar, remaining work and historical branches, the owner decisions of 2026-09-28, the queued investigations), last updated around 2026-09-30. Moved here, unchanged, on 2026-11-07 when the 223 KB file was split (lines 2449-2623 of the file at `a9dee7b2c`; `git show a9dee7b2c:docs/STATUS_AND_HANDOFF.md` returns the unsplit text). The only edit is that relative links gained a `../` prefix so they still resolve from this directory. Every statement below carries its own date and is a record of that day, not a description of today.

---

## 1. Master, and what it has been verified to do

Every promotion to master passed the merge bar (`scripts/bench/box/merge_check.sh`): all `kf-*` crate
tests, v3 gates 9/9, a kf3 build of that exact revision, and the 30-arm thin-guest suite 30/30.
Latest completed bar: **`3f67ed95`** (2026-09-30) — **1742 tests / 0 failed**, gates **9/9**, kf3 build,
bare-metal suite **30/30**, fresh fast guest, thin suite **30/30** on the rerun (29/30 first run, §4.9),
display lane pixel-exact; RTX 3060 (GA106), host 580.159.04; `traces/v3_cifix/`. Only evidence
follows that code on master. New code candidates require their own bar. The bar before it was the
recovered integration **`d883d0eb`** (1683 / 0, 9/9, 30/30; `traces/recovered_integration_20260929/`),
which included the recovered Turing code but was tested on GA106 only. ★ Turing since: the current
master's code passed the same bar on a TU116, plus the CUDA ladder (`traces/v3_turing_master/`).

| Area | State (hardware-measured unless marked) | Doc |
|---|---|---|
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30, **AD104 (RTX 4000 Ada) at master 2026-09-30: bar + ladder 4/4**; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder, **GB205 (RTX 5070) at master 2026-09-30: bar + ladder 4/4** (`traces/v3_families_master/`); floor-swept RTX 3060 Ti 30/30. TU116 (GTX 1660 SUPER) at master `1915bd71` (code `3f67ed95`, 2026-09-30): tests 1742/0, gates 9/9, bare 30/30, thin 30/30, CUDA ladder host 4/4 + guest 4/4 (`traces/v3_turing_master/`). GA100, Hopper, GB10x remain source-derived only; GA100/GB10B refused by name | [Turing recovery evidence](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md), `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| Per-VM GPU UUID | `gpu-uuid=auto\|random\|host\|<GPU-...>` + `vm-id=`: one value for every site, so two VMs on a chip row no longer report one id. **GPU-free tests only; not run in a guest.** Owner decisions open (VM identity, no-identity fallback, slot byte) | `design/V3_GPU_UUID.md` |
| CUDA apps | **61/65 + 6/6 probes at `2830988f`** (`v3-cdp`, host 71/71; CDP fixed, merged as `afb552ea`); the four remaining failures are UVM demand paging (guest fault plane in progress on `v3-uvm-guest`). Earlier: 60/65 at `4c48ca0c`, 100/100 processes; R4 (RTX 3070, `738c90e5` pre-CDP-fix, OFF only): 60/65 + 6/6, = R3 | `design/V3_APP_MATRIX.md` §R3, §R4 |
| Graphics / video | nvkvm-pv's headless graphics set + 15 more items: **38/38** on an RTX 3070 (31 byte-identical to bare metal; OFA optical flow advertised); NVENC/NVDEC byte-exact. Per-call GPU waits are slow on nested boxes (`glFinish` 62 vs 9 µs). R4 (`738c90e5`): bare metal 38/38 only, guest not re-run | `design/V3_GFX_TESTSET.md` (display-phase list §7), `V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins; **every host map snoops the CPU cache** (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE` — without it a CE read stale DRAM on bare-metal hosts; nested VM boxes hid it) | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md`, `traces/v3_adasys/FINDING.txt` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | ★ (2026-09-27, the `v3-drivers` code on master since `5018bb57`) CUDA ladder 4/4 for guests 580.159.04 / 580.105.08 / 590.48.01 / 595.84 / 575.57.08 / 610.57.04; hosts 575.57.08 / 580.95.05 / 580.65.06 gates 9/9, thin 30/30, ladder 4/4, mixed pairs 4/4 — measured on `v3-drivers` heads before the merge (grid §6.0, derived from `traces/driver_matrix/walk/`); 535/545 capability rows approved, independently audited and merged (§3.1); no 535/545 end-to-end application claim. Earlier: 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
| LLM | ★ (2026-09-30, found in the R3 evidence) **llama.cpp: 0.92× host decode, 0.96× prefill** (Qwen2.5-1.5B Q4_K_M, `llama-bench`, nested RTX 3060, trapped doorbells, kf3 `4c48ca0c`; ~4 600 doorbells for ~200 tokens + prefill). **PyTorch eager, Qwen2-0.5B: 0.29×** (0.32× fast path) — the worst case, ~1 084 doorbells per token. Per-token doorbell cost dominates only launch-per-op workloads | `design/V3_BUILD.md`, `V3_GUEST_DOORBELL_MODULE.md` |
| Hardware boundary | every hardware constant pinned to ogkm headers by 43 GPU-free tests; generator `tools/derive_hwref.sh` | `design/V3_HW_BOUNDARY_INVENTORY.md` |
| Pre-v3 tree | archived under `archive/`; 16 `kayfabe-*` crates kept only because the 30-arm grader uses them | `archive/README.md` |

## 2. Remaining work and historical branches

The recovery inventory linked in §0 is authoritative for the unmerged Claude/Turing work.
Do not restart completed investigations from the historical table below. The current scoped
candidate is `codex/allowlist-535-545-audit`; its policy extension is approved and independently
audited, not held for an unanswered owner decision. The 30-commit Claude line and recovered Turing
line remain separately preserved and need integration/testing; their old results are not a
combined-head certification. CDP child execution and display step 1 remain substantive follow-ups.

Historical branch table from the earlier pause:

| Branch | What it is | State / what it needs |
|---|---|---|
| `v3-drivers` | driver matrix (both axes) | everything except its last commit is merged up to `bcd55198`; **last commit `ee35ca4a` = 535/545 capability allowlist — HELD for owner review**; walk continues (575/590/595 guests pass the ladder; 570/565/550 blocked by §4.1) |
| `v3-initrace` | the adapter re-init race + failed-boot recovery | in progress; see §4.1 |
| `v3-adasys` | RTX 4070 sysmem copy failure, bare metal first | in progress; see §4.2 |
| `v3-uvm-n4` | UVM demand-paging research + N4 experiments E5/E6/E6′ | research; E6″ not run; see §3.2 |
| `v3-mgpu-audit` | original multi-GPU audit doc | **superseded** by the version merged with the multi-GPU fix |

## 3. Decisions resolved by the owner — 2026-09-28

### 3.1 The 535/545 capability allowlist (`a50265f8` on `v3-drivers`) — approved

The owner approved the scoped extension and the recommended audit/test work. The shared groups
not covered by the original header sweep have now been checked; the full resolved 550+ policies,
including names/IDs and rule/deny behavior, match the pre-change baseline. Merge only after
the exact candidate passes the normal bar. `ee35ca4a` in older references was a pre-rebase name.

Ports nvproxy's v535_104_05 / v545_23_06 capability rows so those guests get a capability surface
instead of a realize refusal. Existing tables are unchanged (pinned by test). Review points: every
withheld entry is also measured absent from that version's headers; `NVC36F_CTRL_GET_CLASS_ENGINEID` is
allowed at 535/545 (as at 550); the shared-floor rows (`NV00FD`, `NV9096`, `NV906F`, `NV208F`,
`NV90E6`, conf-compute, semaphore surface) now also have exact-tag NVIDIA compiler evidence. Full table:
`design/V3_DRIVER_MATRIX.md` §8.2 ruling 5.

### 3.2 UVM demand paging — route and next experiment

**Selected: b3**, a narrowly scoped opt-in patch to the host's open nvidia-uvm for guest fault
handling. **Full host CUDA coexistence is a requirement**, not a feature to trade away. The VMM
remains unprivileged and untrusted. Preserve ordinary UVM behavior for non-opted-in address spaces.

**N4 is not the next experiment.** Correction to the earlier "host CUDA on that GPU" claim:
580.159.04's `nvUvmInterfaceRegisterUvmCallbacks` has one global registrant, so the replacement
module excludes stock UVM throughout the same host kernel, including a second GPU. Historical N4
research through `v3-uvm-e6pp` (`c6765f5c`) remains useful source evidence. Phase 0 traced a stock-UVM
launch; it did not demonstrate replacement-module replay, cancellation or fault latency.

The next bounded milestone is a **host-only b3 proof**: authenticate ownership, deliver a real
replayable compute fault, map/repair and replay to correct data, cancel only the intended channel,
reject forged/stale handles, bound timeout/teardown, and run ordinary host CUDA concurrently.
The `DupAddressSpace` / `RetainChannel` helper calls must not be assumed to authenticate the caller;
the research's ownership correction applies to b3 too. Production guest fault injection follows
the privileged proof, not the other way around. No route-selection answer is pending from the owner.

### 3.3 The guest doorbell module — how to proceed

**Selected: non-nested baseline and cheaper host-side exits first; optional helper afterward.**
`ioeventfd` remains in the pipeline. Measure vCPU return latency independently of delayed GPU
notification, with idle/synchronous and deep-queue workloads plus content/progress checks. Faster
vCPU return can help throughput without improving single-launch latency. Coalescing is valid only
with preserved ordering and no lost wakeups. **Owner refinement: no batching timers or intentional
delay; act immediately and coalesce only already-pending notifications incidentally.** The optional modified guest NVIDIA driver remains open
for later; it is not selected now. The helper remains design-only with stock-guest fallback.

The owner's ~0.70x bare-metal expectation and possible Windows batching advantage are hypotheses.
Record nesting, same-host controls, doorbells per token, CPU use and correctness hashes; distinguish
a guest kernel entry from a hardware VM exit. Windows performance needs its own measurement.

### 3.4 The execd PSK on `vx` boxes and "no secrets on a box" (raised 2026-09-27)

Cloud sessions have no SSH egress, so they drive boxes through execd behind a cloudflared quick tunnel
(the `vx` kit; `scripts/bench/box/README.md`, last section). The v3-mc20 bar ran this way. Each such box
holds execd's HMAC pre-shared key (`/root/.execd_key`, written by the onstart). Root on a hostile box
can read that key, and the key runs commands as root on every box that holds the same key. With the
kit's per-container token, those are the boxes one session rented. With a shared `$VAST_EXEC_PSK` (set
so that boxes can pass between sessions), they are every box rented with that key. The key opens no
vast account and no GitHub. Ruling F says "no secrets on a box", and security-policy changes need the
owner. The question: is a box-scoped key like this allowed? If yes, may the key be shared across
sessions? If no, what should replace it (for example, a key per box derived from the session's key and
the instance id)? Until the owner rules, the README asks for the per-container token unless a box
must pass to another session.

## 4. Queued / in-progress investigations

1. ~~Adapter re-init race~~ **ANSWERED + FIXED, ON MASTER since `08bf7f18`** (v3-mc19; bar 1637 / 0,
   gates 9/9, 30/30 — see §0). Its evidence is on master too: `traces/v3_initrace/`, and
   `traces/driver_matrix/walk/` since `5018bb57`; `scripts/drivermatrix/initflake_evidence.sh` was on
   master before either, since `36ef0358` (2026-09-26), so "evidence on `v3-drivers`" below was only
   half right. The 570
   guest's next wall is a different one (the UVM first channel: `design/V3_DRIVER_MATRIX.md` stop note, step 1).
   **Adapter re-init race — ROOT-CAUSED + FIXED on `v3-mc19` (uncleared vidmem USERD; see §0).** Original note: Guest 570.148.08 reproduces it every boot (fat guest,
   no persistence mode): on re-init the CeUtils channel is reborn with the same token, host channel id
   and GPFIFO VA; the ring reader yields no words, nothing is pushed, yet the completion tail retires the
   entry (GP_GET authored 1) — the guest sees its work "done" and times out. Rare on 580.x (3/≈270
   boots). A variant: completion observed but the guest CPU read back the wrong data ⇒ suspect the view
   path. Fix rule: an entry that yields no words must wait or refuse — never retire. Also: after one failed
   GSP boot every retry fails on TU/GA10x/Ada (WPR-end margin not modelled). Branch `v3-initrace`;
   evidence on `v3-drivers` (`traces/driver_matrix/walk/`, `scripts/drivermatrix/initflake_evidence.sh`).
2. ~~RTX 4070 sysmem copies fail~~ **ANSWERED + FIXED, ON MASTER (merge `8b32178f`, v3-mc18; promoted as
   `6ec7ec1a`)**: kayfabe's missing CACHE_SNOOP bit, not the box (`traces/v3_adasys/FINDING.txt`).
   Lesson: run memory-plane changes on a non-VM host too.
3. **Hardware gaps** ranked in `design/V3_HW_BOUNDARY_INVENTORY.md` §5.2 (GB100/GB102 PCIe capability,
   VER3 unmapped-big-PTE and sparse-PDE encodings, GB10x 256 GiB leaf, 128 KiB pages, …).
4. **Host GPU wedge after QEMU exit** seen once on an RTX 3090 (GFW boot "progress 0xff", recovered by
   FLR). Watch for recurrence — if the VMM can leave the host GPU unusable, that is a host-side defect.
5. **Turing on hardware** (owner: later, if time).
6. ~~**`cuda/walk` tidy-up**~~ **DONE 2026-09-28, on master:** one `offset_of!` decoder per walk-report
   struct, `read_struct` gone (`17009a4f`); run-flag bit ranges named after `kf_walk.h`, no magic
   shifts at the readers (`30d0ee77`); the VER2/VER3 descriptors are checked field by field against
   each die group's `dev_mmu.h` (`kf-mem/tests/walk_format_vs_ogkm.rs`). What remains is VER3
   (Hopper/Blackwell) semantics, not tidiness: the PDE PCF sparse encodings and the unmapped-big-PTE
   case (`design/V3_HW_BOUNDARY_INVENTORY.md` rows for VER3; item 3 above).
7. Re-run the full CUDA app matrix at the current master (the 58/65 predates several fixes).
8. **Display** (started: M0 on `v3-display`, in the v3-mc21 candidate — §0, §2), then a desktop, then
   **Windows** (roadmap).
9. ⊘ **ROOT-CAUSED 2026-09-30 — not the killed VMM: a kf3 startup race.** That boot's QEMU log says
   `no fd-backed guest RAM block` at t=0.057 s, and then refuses a sysmem leaf of the CeUtils VA space
   by name (`map 0x120070000: guest-RAM row and no RAM object`). Its invalidate stayed unreconciled, so
   RM's `memmgrTestCeUtils` (vid→sys copy through CeUtils) never ran. It is the only one of the 60 boots
   of both runs with that refusal; all 59 others built the 2 GiB RAM object. Mechanism:
   `prewarm` checks for the fd, then `guest_ram_object` looked it up again inside a `OnceLock`.
   QEMU's memory listener re-renders guest RAM at reset (delete, then add), and the second lookup
   hit that gap. The `OnceLock` then cached "no RAM" for the VM's life; the two retries took 2 and
   0 µs, i.e. the cached error, where a real import takes ~1.8 s. Fix on branch **`v3-ramobj`**
   (`once_after`: the fd is read once, outside the cell; "not yet" is never cached; prewarm retries
   next tick) — ★ **MERGED 2026-09-30** after its bar at `f442980e` on TU116 (1744/0, 9/9, bare 30/30,
   thin 30/30; `traces/v3_ramobj/`). Residual: other `RamMap` readers can still observe a
   mid-transaction topology (a refusal by name, not a cached one); fixing that needs the
   listener's `begin`/`commit` staging in `kf3.c`. The text below is the original,
   superseded hypothesis.
   ⊘ *Superseded:* **CeUtils timeout on the first guest init after a killed VMM** (2026-09-30, `traces/v3_cifix/`, seen once).
   At 16:37:56Z a stray QEMU (a kf3 binary of `bf6e7640` left by an earlier mis-launched chain) was
   killed on the RTX 3060 (`kill`, then `kill -9`). The merge check started at 16:38; its bare-metal
   suite used the host GPU in between and passed 30/30. The thin suite's first arm, `--timer`, was the
   first kf3 guest boot after the kill, and it timed out: guest `memmgrMemSet` returned NV_ERR_TIMEOUT
   at 150.7 s, then `ce_utils.c:349` asserted (`lastCompletedPayload == lastSubmittedPayload`). The
   other 29 arms and a full 30-arm rerun passed. Open question: can state left by a killed VMM stall
   the next guest's first CeUtils completion, even though host-side clients work? If so, it is a
   host-side defect in the same class as §4.4. First step: kill a VMM mid-arm, then boot one arm.
10. **CDP on Hopper/Blackwell hardware** (2026-09-30): the SYSTEM_NON_COHERENT SKED-reflected page
    (`kf_chip::sked`) is covered by GPU-free tests only; run `scripts/cdp/cdp_guest.sh` on a GB20x box.
11. **A killed hung guest CUDA process poisons the boot** (`design/V3_CDP.md` §6): after a pre-fix CDP hang
    was SIGKILLed, every later process in that boot failed `cudaSetDeviceFlags` with 999. Not diagnosed;
    the bare-metal control (kill a hung process, then start another) has not been run. If bare metal
    recovers, it is a kayfabe teardown gap (bare-metal pass + guest fail ⇒ kayfabe bug).
12. **Instrument:** `boot_capture.sh`'s host-dmesg line-count watermark reads 0 once a long-lived box's
    kernel ring is full, so "host Xid 0" counts from such boxes were never measured. `scripts/cdp/`
    uses `journalctl -k --since` instead; port that to `boot_capture.sh`.

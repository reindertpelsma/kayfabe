# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-09-27** (written at master `3d523b43`). The single entry point for resuming work
without any chat history. Decisions live in `docs/OWNER_RULINGS.md`; per-topic detail in the design docs
named below. ⊘ When this file and a design doc disagree, the design doc's dated STATUS wins — then fix
this file.

## 1. Master, and what it has been verified to do

Every promotion to master passed the merge bar (`scripts/bench/box/merge_check.sh`): all `kf-*` crate
tests, v3 gates 9/9, a kf3 build of that exact revision, and the 30-arm thin-guest suite 30/30.
Last bar: `bcd55198` — 1616 tests / 0 failed, gates 9/9, 30/30 on an RTX 3060 (GA106).

| Area | State (hardware-measured unless marked) | Doc |
|---|---|---|
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder; floor-swept boards (RTX 3060 Ti) 30/30. Turing, GA100, Hopper, GB10x: **source-derived only** (GA100/GB10B refused by name) | `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md`, `V3_HW_BOUNDARY_INVENTORY.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| CUDA apps | 58/65 nvkvm-pv apps at `670bd310`; fixes since (clpeak, torch_ai_bench, gpu_burn, BAR1-view leak) ⇒ expected ~61/65, **not re-measured**; the rest need UVM demand paging | `design/V3_APP_MATRIX.md` |
| Graphics / video | headless Vulkan, EGL, Xvfb+VirtualGL bit-identical to bare metal; NVENC/NVDEC byte-exact | `design/V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
| LLM | decode ~0.29–0.31× host on nested vast boxes; the gap is mostly doorbell VM exits | `design/V3_BUILD.md`, `V3_GUEST_DOORBELL_MODULE.md` |
| Hardware boundary | every hardware constant pinned to ogkm headers by 43 GPU-free tests; generator `tools/derive_hwref.sh` | `design/V3_HW_BOUNDARY_INVENTORY.md` |
| Pre-v3 tree | archived under `archive/`; 16 `kayfabe-*` crates kept only because the 30-arm grader uses them | `archive/README.md` |

## 2. Branches with work not on master (as of this writing)

| Branch | What it is | State / what it needs |
|---|---|---|
| `v3-drivers` | driver matrix (both axes) | everything except its last commit is merged up to `bcd55198`; **last commit `ee35ca4a` = 535/545 capability allowlist — HELD for owner review**; walk continues (575/590/595 guests pass the ladder; 570/565/550 blocked by §4.1) |
| `v3-initrace` | the adapter re-init race + failed-boot recovery | in progress; see §4.1 |
| `v3-gfxset` | nvkvm-pv's headless graphics test set on kayfabe | in progress; one finding so far: `0x20801210` refused breaks ffmpeg's Vulkan device (the refusal audit had it low-impact) |
| `v3-adasys` | RTX 4070 sysmem copy failure, bare metal first | in progress; see §4.2 |
| `v3-uvm-n4` | UVM demand-paging research + N4 experiments E5/E6/E6′ | research; E6″ not run; see §3.2 |
| `v3-mgpu-audit` | original multi-GPU audit doc | **superseded** by the version merged with the multi-GPU fix |

## 3. Decisions waiting on the owner

### 3.1 The 535/545 capability allowlist (`ee35ca4a` on `v3-drivers`)

Ports nvproxy's v535_104_05 / v545_23_06 capability rows so those guests get a capability surface
instead of a realize refusal. Existing tables are unchanged (pinned by test). Review points: every
withheld entry is also measured absent from that version's headers; `NVC36F_CTRL_GET_CLASS_ENGINEID` is
allowed at 535/545 (as at 550); several shared-floor rows (`NV00FD`, `NV9096`, `NV906F`, `NV208F`,
`NV90E6`, conf-compute, semaphore surface) come from nvproxy alone. Full table:
`design/V3_DRIVER_MATRIX.md` §8.2 ruling 5.

### 3.2 UVM demand paging — route and next experiment

Needed by the remaining CUDA app failures (managed memory touched on demand; GPU access to pageable
memory). Two feasible routes, both needing a privileged host piece for UVM only:
**b3** — a maintained patch to the host's open nvidia-uvm; **N4** — a separate kayfabe host module on
nvidia.ko's exported UVM interface, nvidia-uvm not loaded (host loses CUDA on that GPU; the walker must
launch without libcuda). The owner prefers a separate module over a patch (maintainability).
**E6″ — the open problem, not yet run:** demonstrate that a replayable fault from a compute kernel,
running in a VMM-created fault-capable address space, reaches the module, and measure it (latency
median/p99, replay → correct data, scoped cancel, negative control). E6′ stopped because (1) mapping
memory into that externally-owned address space failed on both the GPU side (`0x33`) and the CPU side
(`0x1f`), and (2) no compute kernel can be launched there without libcuda (nvidia-uvm unloaded). Detail:
`docs/design/V3_UVM_DEMAND_PAGING.md` §11–§13 on `v3-uvm-n4`. The E6″ brief is to come from the owner.

### 3.3 The guest doorbell module — how to proceed

The design is approved (`design/V3_GUEST_DOORBELL_MODULE.md`); nothing is built. Options on the table:
build it as designed; a paravirtual doorbell interface in an optional guest driver build instead of
intercepting mappings; or first make the host-side exit cheaper (in-kernel handling, coalescing) and
measure on a non-nested host, where exits are a few µs rather than ~50 µs.

## 4. Queued / in-progress investigations

1. **Adapter re-init race (a forged completion).** Guest 570.148.08 reproduces it every boot (fat guest,
   no persistence mode): on re-init the CeUtils channel is reborn with the same token, host channel id
   and GPFIFO VA; the ring reader yields no words, nothing is pushed, yet the completion tail retires the
   entry (GP_GET authored 1) — the guest sees its work "done" and times out. Rare on 580.x (3/≈270
   boots). A variant: completion observed but the guest CPU read back the wrong data ⇒ suspect the view
   path. Fix rule: an entry that yields no words must wait or refuse — never retire. Also: after one failed
   GSP boot every retry fails on TU/GA10x/Ada (WPR-end margin not modelled). Branch `v3-initrace`;
   evidence on `v3-drivers` (`traces/driver_matrix/walk/`, `scripts/drivermatrix/initflake_evidence.sh`).
2. **RTX 4070 (AD104) sysmem copies fail** in v3 gates 3/4 on a container box (vidmem fine) — being
   confirmed on bare metal first (`v3-adasys`).
3. **Hardware gaps** ranked in `design/V3_HW_BOUNDARY_INVENTORY.md` §5.2 (GB100/GB102 PCIe capability,
   VER3 unmapped-big-PTE and sparse-PDE encodings, GB10x 256 GiB leaf, 128 KiB pages, …).
4. **Host GPU wedge after QEMU exit** seen once on an RTX 3090 (GFW boot "progress 0xff", recovered by
   FLR). Watch for recurrence — if the VMM can leave the host GPU unusable, that is a host-side defect.
5. **Turing on hardware** (owner: later, if time).
6. **`cuda/walk` tidy-up:** Rust decodes `KfMapRun` by hand-written byte offsets in places (use one
   decoder per `#[repr(C)]` struct via `offset_of!`); name the per-format bit ranges after `dev_mmu.h`.
7. Re-run the full CUDA app matrix at the current master (the 58/65 predates several fixes).
8. **Display**, then a desktop, then **Windows** (roadmap).

## 5. How work was run (so it can be run again)

- **Boxes:** `scripts/bench/box/README.md` — rent, register with the idle watchdog
  (`vast_reaper.sh`), provision (`provision_full.sh`), check (`merge_check.sh`), destroy.
- **Merging:** agents push sub-branches; a merge branch `v3-mcN` combines verified work on the current
  master; the full bar runs on a box at that exact revision; then master and v3 are fast-forwarded. Only
  docs/traces may be added on top of a verified revision without re-running the bar.
- **Local host:** shared and small; builds go on boxes; local cargo only under
  `flock /tmp/claude-0/kf-local-cargo.lock`, 2 jobs, own target dir deleted after.

## 6. Lessons from this campaign worth keeping

- **A refusal on a wait/flush path is a forged completion.** Symptom: results faster than bare metal
  (clpeak "FP64 450 GFLOPS" vs 218). Audit non-OK statuses against a bare-metal trace (nvdiff) first.
- **An entry retired without its work executing is also a forged completion** (§4.1).
- **Merges combine branches that each passed alone** — two branches added a same-named field with
  different meanings; one branch left another's test red; two bumped the same ABI number. Resolve
  semantically and re-run the whole bar on the merged revision.
- **A test runner without `--no-fail-fast` under-reports** (it stopped at the first red crate).
- **A watchdog must not read "cannot probe" as "idle".**
- **Boxes vanish** (vast destroyed several mid-run); evidence must already be in git.

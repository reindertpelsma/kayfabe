# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-09-27** (updated ~19:40 UTC at master `db038f5f` = v3-mc20 and merge candidate
`4c48ca0c` = v3-mc21; ⊘ before that, updated at master `6ec7ec1a` when work paused at the weekly usage
limit — see §0). The single entry point for resuming work
without any chat history. Decisions live in `docs/OWNER_RULINGS.md`; per-topic detail in the design docs
named below. ⊘ When this file and a design doc disagree, the design doc's dated STATUS wins — then fix
this file.

## 0. Where work stands — resume here (updated 2026-09-27 ~19:40 UTC; work first paused 2026-09-27 ~00:05 CEST)

- **2026-09-27 19:59 UTC — `v3-mc21` (= master `db038f5f` + `origin/v3-display`, merge `4c48ca0c`) PASSED THE
  FULL MERGE BAR** at exactly `4c48ca0c`: crate tests **1651 / 0**, gates **9/9**, `KF3_RC=0` (VNC+pixman build),
  `FG_RC=0`, fast suite **30/30** — same box. Evidence: `traces/v3_mc21/`. Display stays default-off.
  **Master and v3 fast-forwarded to `8ab92bf4`** (= `4c48ca0c` + evidence/docs; owner go-ahead in the session,
  2026-09-28). The `display=on` M0 lane on GA106 (`traces/v3_display/mc21m0/`) reproduces GA102's `m0a`
  exactly: KernelDisplay up, NVKMS stops at `0x730101`/`0x730102`/`0x730107`/`0x730151` ⇒ displayless.
- **Boxes from a cloud session:** the container has no SSH egress, so a box is driven through the **`vx`
  kit** (execd behind a cloudflared quick tunnel). `provision_full.sh` and `merge_check.sh` run unchanged
  that way; `traces/v3_mc20/README.md` records the v3-mc20 bar run like this.
- **`v3-uvm-e6pp` (`c6765f5c`): E6″ phase 0 ran** (a capture on a stock box — no fault numbers yet); §2, §3.2.
- ⊘ *Superseded 2026-09-28 (master is now `8ab92bf4`, above; v3-mc21 passed):* **2026-09-27 19:38 UTC — master
  and v3 = `db038f5f`** (= `c0ef7b75`, the revision that passed the bar below, + the commit that added its
  evidence, `traces/v3_mc20/` and this file only), fast-forwarded with the owner's go-ahead. `v3-drivers~1`
  (`8f9bdd14`) is therefore **ON MASTER** (via merge `5018bb57`); `v3-drivers` now holds only the held 535/545
  allowlist commit, **`a50265f8`** (§3.1). Merge candidate `v3-mc21` = `4c48ca0c` … bar in progress.
- **2026-09-27 18:27 UTC — `v3-mc20` PASSED THE FULL MERGE BAR** at `c0ef7b75` (= `5018bb57` + docs only), on
  branch `claude/kayfabe-gpu-testing-m0cv1q`: crate tests **1639 / 0**, gates **9/9**, `KF3_RC=0`, `FG_RC=0`,
  fast suite **30/30** — vast 53004208, RTX 3060 GA106, host 580.159.04 open. Evidence: `traces/v3_mc20/`.
  ⊘ *2026-09-27 19:38 UTC: done; see the first line.* Ready to fast-forward master and v3 (owner go-ahead
  asked in the session). Cloud sessions now reach boxes through execd behind a cloudflared quick tunnel
  (the `vx` kit), not SSH.
- ⊘ *Superseded by the line above:* 2026-09-27, resumed (cloud session): merge candidate `v3-mc20` = master `0d3ecde9` + `v3-drivers~1` (`8f9bdd14`,
  the held 535/545 allowlist commit left out) — merge commit `5018bb57`, clean, on branch `claude/relaxed-babbage-7zz89s`.
  GPU-free: all `kf-*` crate tests **1639 passed / 0 failed** at `5018bb57`. Hardware bar (gates, kf3 build, 30/30)
  NOT run: this session's container has no SSH egress, so `merge_check.sh` cannot be driven on a box from here.
  Do not promote to master until the bar passes at `5018bb57`.
- ⊘ *Superseded 2026-09-27 19:38 UTC — master is now `db038f5f` (v3-mc20), see the first line:*
  **Master = `v3-mc19` verified** (`08bf7f18`: 1637 tests / 0 failed, gates 9/9, 30/30) + docs. All vast boxes are being
  destroyed; nothing depends on a box or on local files. Every branch below is on GitHub.
- **`v3-initrace` is ON MASTER** (verified at `08bf7f18`). It contains: USERD cleared at Translated-channel birth (the re-init "flake": a reborn CeUtils channel
  inherited a leftover GP_PUT — 0/300 after, 20/20 injected failures before) and WPR2 served at the guest's
  own FWSEC-FRTS offset after a failed GSP boot (retry boots; 20/20 later opens pass).
- ⊘ *Merged 2026-09-27: `8f9bdd14` is on master via v3-mc20 (`5018bb57`; bar at `c0ef7b75`); the held
  allowlist commit is now `a50265f8`, the only commit on `v3-drivers` beyond master. The "Next for that
  branch" items below still stand:*
  **`v3-drivers` mergeable head `8f9bdd14`** (= `42b25354` + a default-off logging aid + its stop note; the bar was
  last fully green at `1837166d`, the thin suite at the newest head never ran) (rebased on `6ec7ec1a`; the 535/545 allowlist commit is LAST and held
  for owner review): guest 610 ladder 4/4, hosts 575.57.08 / 580.95.05 / 580.65.06 green on every row,
  (runlist, chid) token index, ≤545 GET_CHIP_INFO carry, 595+ GSP heartbeat. Merge it onto the new master
  after `v3-mc19`, then run the bar. Next for that branch: the 570 guest's UVM-first-channel wall (token
  0x803 reads its GPFIFO before the mirror maps it) and a 570 thin-guest NULL deref seen on master.
- ⊘ *2026-09-28: ON MASTER — merged as v3-mc21 `4c48ca0c`, bar passed (§0 first line), master `8ab92bf4`; step (1)
  is wired in code since (`docs/design/V3_DISPLAY.md` step-(1) note); superseded text follows:*
  **`v3-display` (`adbea28e`)**: Phase 1 decided — emulate the display hardware the stock guest driver expects
  (~3–5 weeks to a desktop on GA10x); M0 (behind `display=on`, default off) brings the guest's display layer
  up; next step and plan in the stop note at the top of `docs/design/V3_DISPLAY.md`. Merge bar not run.
- **Open owner decisions**: §3 (535/545 allowlist `a50265f8`, UVM route + E6″ brief, doorbell module, execd PSK
  on `vx` boxes).

## 1. Master, and what it has been verified to do

Every promotion to master passed the merge bar (`scripts/bench/box/merge_check.sh`): all `kf-*` crate
tests, v3 gates 9/9, a kf3 build of that exact revision, and the 30-arm thin-guest suite 30/30.
Last bar: **`c0ef7b75`** (v3-mc20, 2026-09-27) — **1639 tests / 0 failed**, gates **9/9**, `KF3_RC=0`,
thin suite **30/30** on an RTX 3060 (GA106), vast 53004208, host 580.159.04 open; evidence
`traces/v3_mc20/`. Master = `db038f5f` = that revision + its evidence commit. (The bar does not run the
CUDA ladder or the fat-guest lanes.)
⊘ *Superseded by the line above, kept for the record:* Last bar: `f89f66bb` — 1625 tests / 0 failed,
gates 9/9, 30/30 on an RTX 3060 (GA106). (Between the two, `6ec7ec1a` (v3-mc18) and `08bf7f18` (v3-mc19,
1637 / 0) were promoted the same way. Their merge-check logs are `mc18*` and `mc19*` in
`traces/vh_archive/vmc_mergecheck_logs.tgz`. No log of the `f89f66bb` (v3-mc17) run is in the repo:
`vh_mergecheck_logs.tgz`'s `mc17.log` is an earlier run, on the vh box at `5d2b0a33`.)

| Area | State (hardware-measured unless marked) | Doc |
|---|---|---|
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder; floor-swept boards (RTX 3060 Ti) 30/30. Turing, GA100, Hopper, GB10x: **source-derived only** (GA100/GB10B refused by name) | `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md`, `V3_HW_BOUNDARY_INVENTORY.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| CUDA apps | 58/65 nvkvm-pv apps at `670bd310`; fixes since (clpeak, torch_ai_bench, gpu_burn, BAR1-view leak) ⇒ expected ~61/65, **not re-measured**; the rest need UVM demand paging | `design/V3_APP_MATRIX.md` |
| Graphics / video | nvkvm-pv's headless graphics set + 15 more items: **38/38** on an RTX 3070 (31 byte-identical to bare metal; OFA optical flow advertised); NVENC/NVDEC byte-exact. Per-call GPU waits are slow on nested boxes (`glFinish` 62 vs 9 µs) | `design/V3_GFX_TESTSET.md` (display-phase list §7), `V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins; **every host map snoops the CPU cache** (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE` — without it a CE read stale DRAM on bare-metal hosts; nested VM boxes hid it) | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md`, `traces/v3_adasys/FINDING.txt` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | ★ (2026-09-27, the `v3-drivers` code on master since `5018bb57`) CUDA ladder 4/4 for guests 580.159.04 / 580.105.08 / 590.48.01 / 595.84 / 575.57.08 / 610.57.04; hosts 575.57.08 / 580.95.05 / 580.65.06 gates 9/9, thin 30/30, ladder 4/4, mixed pairs 4/4 — measured on `v3-drivers` heads before the merge (grid §6.0, derived from `traces/driver_matrix/walk/`); 535/545 capability rows held (§3.1). Earlier: 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
| LLM | decode ~0.29–0.31× host on nested vast boxes; the gap is mostly doorbell VM exits | `design/V3_BUILD.md`, `V3_GUEST_DOORBELL_MODULE.md` |
| Hardware boundary | every hardware constant pinned to ogkm headers by 43 GPU-free tests; generator `tools/derive_hwref.sh` | `design/V3_HW_BOUNDARY_INVENTORY.md` |
| Pre-v3 tree | archived under `archive/`; 16 `kayfabe-*` crates kept only because the 30-arm grader uses them | `archive/README.md` |

## 2. Branches with work not on master (as of 2026-09-27 ~19:40 UTC, master `db038f5f`)

Checked with `git branch -r --no-merged origin/master`.

| Branch | What it is | State / what it needs |
|---|---|---|
| `v3-display` (`adbea28e`) | display plane: Phase 1 design + M0 (`display=on`, default off) | **ON MASTER** (v3-mc21 `4c48ca0c`, bar passed 2026-09-27; master `8ab92bf4`); step (1) wired in code on `claude/kayfabe-gpu-testing-m0cv1q`, not yet on the bench; GA106 M0 lane `traces/v3_display/mc21m0/`. Next steps: the stop note atop `design/V3_DISPLAY.md`; M0 evidence `traces/v3_display/m0a/` |
| `v3-drivers` (`a50265f8`) | driver matrix (both axes) | everything but its tip is on master (`8f9bdd14` via v3-mc20, `5018bb57`); **the tip `a50265f8` = 535/545 capability allowlist — HELD for owner review** (§3.1). Next: `design/V3_DRIVER_MATRIX.md` stop note — the 570 guest's UVM first-channel wall first |
| `v3-uvm-e6pp` (`c6765f5c`) | E6″ phase 0, forked from `v3-uvm-n4` at `600b864a` | research: **phase 0 ran** — the real libcuda's mapping/launch ioctl shape captured on a stock box (`V3_UVM_DEMAND_PAGING.md` §12.6 on that branch; evidence `traces/v3_uvm_research/e6pp/results.txt` there). E6″ proper (module present; fault, latency, replay, cancel) not run; see §3.2 |
| `v3-uvm-n4` (`600b864a`) | UVM demand-paging research + N4 experiments E5/E6/E6′ (`V3_UVM_DEMAND_PAGING.md` §11–§13; §0–§10 are on master via `v3-uvm-research`) | research; continued on `v3-uvm-e6pp`; see §3.2 |
| `v3-mgpu-audit` | original multi-GPU audit doc | **superseded** by the version merged with the multi-GPU fix |

The same listing also shows older branches last committed 2026-09-24 … 26 (`v3-p3`, `v3-p4a`, `v3-p4b`,
`v3-p5`, `v3-p5b`, `v3-p6uvm`, `v3-walkgraph`, `v3-attrib`, `v3-dbmod`, `v3-family`, `v3-hostfacts`,
`v3-mgpu`, and `v3-uvm-nopatch`, whose §11 is inside `v3-uvm-n4`); this file does not classify them.

⊘ *Superseded 2026-09-27 by the table above — kept as written at master `6ec7ec1a`. Since then
`v3-initrace` went to master at `08bf7f18` (v3-mc19), `v3-adasys` at `8b32178f` (v3-mc18), and
`v3-drivers~1` at `5018bb57` (v3-mc20); `ee35ca4a` below is an earlier (pre-rebase) SHA of the
allowlist change, no longer in the object store — the change is now `a50265f8`:*

| Branch | What it is | State / what it needs |
|---|---|---|
| `v3-drivers` | driver matrix (both axes) | everything except its last commit is merged up to `bcd55198`; **last commit `ee35ca4a` = 535/545 capability allowlist — HELD for owner review**; walk continues (575/590/595 guests pass the ladder; 570/565/550 blocked by §4.1) |
| `v3-initrace` | the adapter re-init race + failed-boot recovery | in progress; see §4.1 |
| `v3-adasys` | RTX 4070 sysmem copy failure, bare metal first | in progress; see §4.2 |
| `v3-uvm-n4` | UVM demand-paging research + N4 experiments E5/E6/E6′ | research; E6″ not run; see §3.2 |
| `v3-mgpu-audit` | original multi-GPU audit doc | **superseded** by the version merged with the multi-GPU fix |

## 3. Decisions waiting on the owner

### 3.1 The 535/545 capability allowlist (`a50265f8`, the tip of `v3-drivers`)

⊘ *2026-09-27: this heading named `ee35ca4a` — an earlier (pre-rebase) SHA of the same change; the
branch was rewritten after it (the walk measured later heads `1837166d` and `bb9b67a9`) and then rebased
onto `6ec7ec1a`. It is now `a50265f8`, the only commit on `v3-drivers` that is not on master. The
"full table" this section points to is in that commit's own edit to `design/V3_DRIVER_MATRIX.md` §8.2
ruling 5 (`git show a50265f8 -- docs/design/V3_DRIVER_MATRIX.md`); master's copy of the doc has the
ruling, not the table.*

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

★ **2026-09-27 — E6″ phase 0 ran** (branch `v3-uvm-e6pp`, `c6765f5c`; `V3_UVM_DEMAND_PAGING.md` §12.6 on
that branch, evidence `traces/v3_uvm_research/e6pp/results.txt` there). On a stock box (nvidia-uvm
loaded, no takeover) an `LD_PRELOAD` logger recorded what the real libcuda sends: every data allocation
is an `RM_ALLOC` (`0x0040` vidmem / `0x003e` sysmem) followed by exactly one `UVM_CREATE_EXTERNAL_RANGE`
+ one `UVM_MAP_EXTERNAL_ALLOCATION`, never a raw `NV_ESC_RM_MAP_MEMORY_DMA` into the VAS — E6′'s `0x33`
used the wrong mechanism; a sysmem allocation adds a CPU-side `NV_ESC_RM_MAP_MEMORY` between the alloc
and that pair — E6′'s `0x1f` was that map with the wrong handle/order. The QMD layout is public
(`open-gpu-doc` `clc7c0qmd.h`): without libcuda what is missing is a compiler, not the layout. A
correction there to §12.3.1(2): `nvGpuOpsDupAddressSpace` / `nvGpuOpsRetainChannel` do not check who
the caller is (the source's own "Bug 1624521" TODOs), so an N4 module must validate the adopting VMM
itself. **Still not measured:** fault delivery, latency, replay, cancel — E6″ proper (module present,
the captured calls driven against the adopted VAS) has not run.

**E6″ — the open problem, not yet run:** (⊘ *2026-09-27: phase 0 has since run — above; E6″ proper has
not.*)
demonstrate that a replayable fault from a compute kernel,
running in a VMM-created fault-capable address space, reaches the module, and measure it (latency
median/p99, replay → correct data, scoped cancel, negative control). E6′ stopped because (1) mapping
memory into that externally-owned address space failed on both the GPU side (`0x33`) and the CPU side
(`0x1f`), and (2) no compute kernel can be launched there without libcuda (nvidia-uvm unloaded). Detail:
`docs/design/V3_UVM_DEMAND_PAGING.md` §11–§13 on `v3-uvm-n4` (+ §12.6, phase 0, on `v3-uvm-e6pp`). The
E6″ brief is to come from the owner.

### 3.3 The guest doorbell module — how to proceed

The design is approved (`design/V3_GUEST_DOORBELL_MODULE.md`); nothing is built. Options on the table:
build it as designed; a paravirtual doorbell interface in an optional guest driver build instead of
intercepting mappings; or first make the host-side exit cheaper (in-kernel handling, coalescing) and
measure on a non-nested host, where exits are a few µs rather than ~50 µs.

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
6. **`cuda/walk` tidy-up:** Rust decodes `KfMapRun` by hand-written byte offsets in places (use one
   decoder per `#[repr(C)]` struct via `offset_of!`); name the per-format bit ranges after `dev_mmu.h`.
7. Re-run the full CUDA app matrix at the current master (the 58/65 predates several fixes).
8. **Display** (started: M0 on `v3-display`, in the v3-mc21 candidate — §0, §2), then a desktop, then
   **Windows** (roadmap).

## 5. How work was run (so it can be run again)

- **Boxes:** `scripts/bench/box/README.md` — rent, register with the idle watchdog
  (`vast_reaper.sh`), provision (`provision_full.sh`), check (`merge_check.sh`), destroy. From a cloud
  session (no SSH egress) the same scripts are driven through the `vx` kit — execd behind a cloudflared
  quick tunnel (the v3-mc20 bar ran this way: `traces/v3_mc20/README.md`).
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
- **`git stash` is shared by every worktree of a repo** — another agent's `stash pop` applied someone
  else's stash. Agents working in parallel worktrees save a patch file instead of stashing.

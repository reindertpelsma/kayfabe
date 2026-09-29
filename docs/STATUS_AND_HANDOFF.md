# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-09-29.** The owner resolved the three implementation decisions in §3;
work resumed, but remote verification/backup is currently blocked by the session's network sandbox.
See `RESUME_2026-09-29.md` for the exact checkpoint. The historical campaign entries below are dated;
the last published product baseline at this resumption is `8ab92bf4` (mc21). The single entry point for resuming work
without any chat history. Decisions live in `docs/OWNER_RULINGS.md`; per-topic detail in the design docs
named below. ⊘ When this file and a design doc disagree, the design doc's dated STATUS wins — then fix
this file.

## 0. Current resumption — start here

- **2026-09-29 checkpoint:** `/workspace/kayfabe` is now on `codex/resume-2026-09-29`, based on
  the GitHub-backed candidate `10b725a6`. The prior recovery branch and the `/data` worktrees
  are preserved. SSH fails with `socket: Operation not permitted`; GitHub/Vast CLI cannot resolve
  their hosts. Do not infer that the boxes disappeared or the final bar failed. Its last observed
  state was **1653/0 tests, gates 9/9, KF3_RC=0, FG_RC=0, 20/30 guest cases passing**, with the
  suite still running at 2026-09-28 18:38:33 UTC. Completion is **unknown**, not promotion-ready.
  No current box inventory, GitHub refresh, push or teardown was possible on 2026-09-29.

- **2026-09-28 owner decisions:** 535/545 capability extension approved, subject to the independent
  audit and exact-revision merge bar; **b3 patched host nvidia-uvm selected, full native host CUDA
  coexistence required**; doorbells: **non-nested baseline and host-side/ioeventfd work first, guest
  helper afterward**, optional modified guest driver deferred. See §3 and `OWNER_RULINGS.md`.
- **Recovery is preserved, not implicitly merged:** source/evidence at
  [`recovery/resume-2026-09-28`](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md),
  recovered Turing code at `recovery/vast-tuwork-2026-09-28` (`2825c42f`, code tree `576f5bb0`).
  The Claude head `826ef957` has 30 commits beyond the baseline and needs its own combined merge bar.
  Do not use those historical results to certify a new candidate. The fresh allowlist candidate
  starts from the published baseline, so it does not silently promote the other pending changes.
- **Physical baseline access:** read-only SSH to `172.22.1.20` reports `Network is unreachable`
  from this workspace on 2026-09-28. Prepare the protocol locally; do not label a Vast KVM VM as a
  non-nested host. No physical-host session or display was changed. Paguro's retained Windows VM
  on instance `53076605` is running and reserved for the separate Paguro chat.
- **Allowlist audit complete:** full 550–610 before/after policy comparison plus compiler-derived
  checks for all 16 admitted controls from the previously unchecked shared groups. Both the
  pre-change golden and the two header fixtures were independently regenerated on the trusted
  development host with byte-identical results. Final merge bar at `61c49f14` needs its terminal
  evidence retrieved (completion currently unknown);
  see `traces/capability_535_545_audit_20260928/README.md` for exact revisions and limitations.
- **Doorbell work is mechanism evidence, not production acceleration:** the GPU-free KVM probe
  verifies token-matched ioeventfd and unmatched MMIO fallback on a read-only memslot. No timers
  or intentional batching delay; initially accelerate passthrough only, retaining the existing
  translated ordering and real-GPU completion paths. The generic Emulated route is not a live
  kf3 channel backend. See `design/V3_DOORBELL_BASELINE.md`.
- **UVM next:** finish the checked registration/lifetime proof described in
  `design/V3_UVM_B3_IMPLEMENTATION.md`, then build the bounded host-only experiment. No b3 patch
  has been loaded. In particular, a UVM channel-memory reference alone does not prevent hardware
  state from being detached and freed; delayed userspace replies need explicit invalidation.
- **Last verified box inventory, 2026-09-28:** `53080587` (GTX 1660 SUPER) was retired after
  rechecking all 493 saved evidence hashes and its recovered source tree. Retained: `53004208`
  (RTX 3060, verification/next Kayfabe tests) and `53076605` (Paguro Windows). No new rentals.
  Combined listed rate was approximately $0.5203/hour before additional fees. They may still
  incur charges; current access is blocked. Preserve final-run evidence before any retirement.
- **Historical pause notes below are superseded by this resumption.** mc21 was promoted to
  `8ab92bf4`; older "awaiting promotion", "all boxes being destroyed", and open-choice lines below
  describe earlier points in the campaign, not current actions.
- **2026-09-27 19:59 UTC — `v3-mc21` (= master `db038f5f` + `origin/v3-display`, merge `4c48ca0c`) PASSED THE
  FULL MERGE BAR** at exactly `4c48ca0c`: crate tests **1651 / 0**, gates **9/9**, `KF3_RC=0` (VNC+pixman build),
  `FG_RC=0`, fast suite **30/30** — same box. Evidence: `traces/v3_mc21/`. Display stays default-off; the
  `display=on` lane on GA106 is the next box job. Promotion of `4c48ca0c` awaits the owner's go-ahead.
- **Master and v3 = `db038f5f`** (fast-forwarded 2026-09-27 with the owner's go-ahead in the session):
  v3-mc20 below + its evidence. `v3-drivers~1` (`8f9bdd14`) is on master; the allowlist `a50265f8` is not.
- **2026-09-27 18:27 UTC — `v3-mc20` PASSED THE FULL MERGE BAR** at `c0ef7b75` (= `5018bb57` + docs only), on
  branch `claude/kayfabe-gpu-testing-m0cv1q`: crate tests **1639 / 0**, gates **9/9**, `KF3_RC=0`, `FG_RC=0`,
  fast suite **30/30** — vast 53004208, RTX 3060 GA106, host 580.159.04 open. Evidence: `traces/v3_mc20/`.
  Ready to fast-forward master and v3 (owner go-ahead asked in the session). Cloud sessions now reach boxes
  through execd behind a cloudflared quick tunnel (the `vx` kit), not SSH.
- ⊘ *Superseded by the line above:* 2026-09-27, resumed (cloud session): merge candidate `v3-mc20` = master `0d3ecde9` + `v3-drivers~1` (`8f9bdd14`,
  the held 535/545 allowlist commit left out) — merge commit `5018bb57`, clean, on branch `claude/relaxed-babbage-7zz89s`.
  GPU-free: all `kf-*` crate tests **1639 passed / 0 failed** at `5018bb57`. Hardware bar (gates, kf3 build, 30/30)
  NOT run: this session's container has no SSH egress, so `merge_check.sh` cannot be driven on a box from here.
  Do not promote to master until the bar passes at `5018bb57`.
- **Master = `v3-mc19` verified** (`08bf7f18`: 1637 tests / 0 failed, gates 9/9, 30/30) + docs. All vast boxes are being
  destroyed; nothing depends on a box or on local files. Every branch below is on GitHub.
- **`v3-initrace` is ON MASTER** (verified at `08bf7f18`). It contains: USERD cleared at Translated-channel birth (the re-init "flake": a reborn CeUtils channel
  inherited a leftover GP_PUT — 0/300 after, 20/20 injected failures before) and WPR2 served at the guest's
  own FWSEC-FRTS offset after a failed GSP boot (retry boots; 20/20 later opens pass).
- **`v3-drivers` mergeable head `8f9bdd14`** (= `42b25354` + a default-off logging aid + its stop note; the bar was
  last fully green at `1837166d`, the thin suite at the newest head never ran) (rebased on `6ec7ec1a`; the 535/545 allowlist commit is LAST and held
  for owner review): guest 610 ladder 4/4, hosts 575.57.08 / 580.95.05 / 580.65.06 green on every row,
  (runlist, chid) token index, ≤545 GET_CHIP_INFO carry, 595+ GSP heartbeat. Merge it onto the new master
  after `v3-mc19`, then run the bar. Next for that branch: the 570 guest's UVM-first-channel wall (token
  0x803 reads its GPFIFO before the mirror maps it) and a 570 thin-guest NULL deref seen on master.
- **`v3-display` (`adbea28e`)**: Phase 1 decided — emulate the display hardware the stock guest driver expects
  (~3–5 weeks to a desktop on GA10x); M0 (behind `display=on`, default off) brings the guest's display layer
  up; next step and plan in the stop note at the top of `docs/design/V3_DISPLAY.md`. Merge bar not run.
- **Owner decisions resolved 2026-09-28**: §3. The cloud-only execd shared-key question remains
  separate; direct SSH without agent forwarding avoids it and does not block this work.

## 1. Master, and what it has been verified to do

Every promotion to master passed the merge bar (`scripts/bench/box/merge_check.sh`): all `kf-*` crate
tests, v3 gates 9/9, a kf3 build of that exact revision, and the 30-arm thin-guest suite 30/30.
Last published bar at resumption: **`4c48ca0c`** (mc21) — **1651 tests / 0 failed**, gates **9/9**,
build and fast guest successful, thin suite **30/30**, RTX 3060 (GA106); `traces/v3_mc21/`.
Published baseline `8ab92bf4` adds only evidence/docs to that tested code. New candidates require a
new bar; this is not a current-run test claim.

| Area | State (hardware-measured unless marked) | Doc |
|---|---|---|
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder; floor-swept boards (RTX 3060 Ti) 30/30. Turing, GA100, Hopper, GB10x: **source-derived only** (GA100/GB10B refused by name) | `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md`, `V3_HW_BOUNDARY_INVENTORY.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| CUDA apps | 58/65 nvkvm-pv apps at `670bd310`; fixes since (clpeak, torch_ai_bench, gpu_burn, BAR1-view leak) ⇒ expected ~61/65, **not re-measured**; the rest need UVM demand paging | `design/V3_APP_MATRIX.md` |
| Graphics / video | nvkvm-pv's headless graphics set + 15 more items: **38/38** on an RTX 3070 (31 byte-identical to bare metal; OFA optical flow advertised); NVENC/NVDEC byte-exact. Per-call GPU waits are slow on nested boxes (`glFinish` 62 vs 9 µs) | `design/V3_GFX_TESTSET.md` (display-phase list §7), `V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins; **every host map snoops the CPU cache** (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE` — without it a CE read stale DRAM on bare-metal hosts; nested VM boxes hid it) | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md`, `traces/v3_adasys/FINDING.txt` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
| LLM | decode ~0.29–0.31× host on nested vast boxes; the gap is mostly doorbell VM exits | `design/V3_BUILD.md`, `V3_GUEST_DOORBELL_MODULE.md` |
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

## 4. Queued / in-progress investigations

1. **Adapter re-init race — ROOT-CAUSED + FIXED on `v3-mc19` (uncleared vidmem USERD; see §0).** Original note: Guest 570.148.08 reproduces it every boot (fat guest,
   no persistence mode): on re-init the CeUtils channel is reborn with the same token, host channel id
   and GPFIFO VA; the ring reader yields no words, nothing is pushed, yet the completion tail retires the
   entry (GP_GET authored 1) — the guest sees its work "done" and times out. Rare on 580.x (3/≈270
   boots). A variant: completion observed but the guest CPU read back the wrong data ⇒ suspect the view
   path. Fix rule: an entry that yields no words must wait or refuse — never retire. Also: after one failed
   GSP boot every retry fails on TU/GA10x/Ada (WPR-end margin not modelled). Branch `v3-initrace`;
   evidence on `v3-drivers` (`traces/driver_matrix/walk/`, `scripts/drivermatrix/initflake_evidence.sh`).
2. ~~RTX 4070 sysmem copies fail~~ **ANSWERED + FIXED (master `6ec7ec1a`)**: kayfabe's missing CACHE_SNOOP
   bit, not the box. Lesson: run memory-plane changes on a non-VM host too.
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
- **`git stash` is shared by every worktree of a repo** — another agent's `stash pop` applied someone
  else's stash. Agents working in parallel worktrees save a patch file instead of stashing.

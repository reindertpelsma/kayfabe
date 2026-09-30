# Status and handoff — where kayfabe v3 stands, and how to resume

**STATUS: LIVE, 2026-09-30.** Master = **`b32aa046`**: the code of `3f67ed95`, which passed the full
merge bar (§0 first entry), plus evidence. The single entry point for resuming work without any chat
history. Decisions live in `docs/OWNER_RULINGS.md` (doorbell refinements of 2026-09-30 in §D); per-topic
detail in the design docs named below. ⊘ When this file and a design doc disagree, the design doc's
dated STATUS wins — then fix this file. Entries below the first are dated history.

## 0. Current resumption — start here

- **2026-09-30 — master and v3 = `b32aa046`.** Landed since `5c639f54`:
  - **CI repair** (`v3-ci`, `bf6e7640`): first green GitHub CI; the hardware bar is now fail-closed and
    includes a bare-metal suite (`scripts/bench/box/merge_check.sh`, run from a repo checkout).
  - **Display M1–M3** (`v3-display2`): the emulated display (`display=on`, default **off**) drives a
    virtual monitor. Pixel-exact 1920x1080 scanout, 120/120 flips at 60 Hz, Mint's Cinnamon Wayland
    desktop with vkcube; weston; Xorg with the NVIDIA driver. X11 Cinnamon/Vulkan need
    `GF100_DISP_SW` (owner choice below). `design/V3_DISPLAY.md`, `traces/v3_display/`.
  - **Doorbell fast path** (`v3-ioeventfd`): one KVM ioeventfd per live token, drained off the vCPU,
    Passthrough rung inline, Translated handed on. Default **off** (`doorbell-ioeventfd=on`). Nested
    boxes only: LLM decode 0.29× → 0.32× of host. `design/V3_DOORBELL_IOEVENTFD.md`.
  - **UVM b3 host-only proof** (`v3-uvm-b3`, tools only): the opt-in nvidia-uvm patch delivers a real
    compute fault to the owning process, which maps the page and replays to correct data, alongside
    native host CUDA. No guest fault plane yet. `design/V3_UVM_B3_IMPLEMENTATION.md`, `traces/v3_uvm_b3/`.
  - **Bars:** `v3-mc22` (display + fast path, KF3 ABI 10) passed at `b84250b8`
    (`traces/v3_mc22/`). Then `3f67ed95` (lint fixes plus claim-ledger citations, no behaviour change;
    GitHub CI run 36744304349 green): tests **1742/0**, gates **9/9**, KF3_RC=0, bare **30/30**,
    FG_RC=0, and the display lane pixel-exact. The thin suite was **29/30** on the first run: `--timer`
    CeUtils timeout on the first boot after a stray QEMU was SIGKILLed on the same GPU (§4.9). It was
    **30/30** on the rerun at the same revision. `traces/v3_cifix/`.
  - **Open owner choices:** (1) build the guest doorbell helper now, or first measure a non-nested
    host? Needs a physical host: `172.22.1.20` was still `Network is unreachable` from this workspace
    on 2026-09-30, and the RTX 3050 kiosk PC's address is not recorded in the repo.
    (2) `GF100_DISP_SW` (X11 desktops): A = allocate a host object for it, which breaks the rule that
    kf-rm refuses unknown classes; B = keep refusing it, which leaves X11 partial. Recommendation: B
    now; try a kayfabe-serviced vblank next; A only with a headless guard. (3) b3 registration
    ownership (NVIDIA Bug 1624521) for mutually untrusting VMMs on one GPU: now or later.
  - **Queued:** the UVM guest fault plane (`design/V3_UVM_DEMAND_PAGING.md` §5, "What the guest must see"); CDP child launch;
    display leftovers (cursor, scaled windows, 16-bit/YUV, the §7 display apps); the rest of the driver
    matrix (570 UVM first-channel wall; hosts 535/590/595/610); re-running the app and graphics matrices
    at this master; Turing on hardware.
  - **CI at `fc2dadc4` (2026-09-30): all three jobs green**, including the **slow** job (the whole suite
    with the soaks, `KAYFABE_SLOW=1`), dispatched as run 36760547434. First green slow run since the CI
    repair; the scheduled runs of 09-28…09-30 had failed at pre-repair revisions.
  - **`v3-ramobj` merged (2026-09-30):** kf3 no longer caches a transient "guest RAM not registered
    yet" for the VM's life — the cause of the one `--timer` failure in the `3f67ed95` bar (§4.9). Bar at
    `f442980e` on TU116: 1744/0, 9/9, bare 30/30, thin 30/30 (`traces/v3_ramobj/`).
  - **Turing at this master (2026-09-30, `traces/v3_turing_master/`):** GTX 1660 SUPER (TU116), merge
    bar at `1915bd71` (code `3f67ed95`): tests 1742/0, gates 9/9, bare 30/30, thin 30/30; CUDA ladder
    host 4/4 and guest 4/4.
  - **Boxes (2026-09-30):** `53004208` (RTX 3060, alias `v3060`, retained; the CDP agent's box);
    `53562843` (GTX 1660 SUPER, alias `vtu`, the `v3-ramobj` bar); plus one box per agent (UVM guest
    plane, app/graphics matrix R4), each registered with the reaper. Nothing on any box is the only
    copy of anything.

- **2026-09-29 CI repair in progress:** `codex/ci-repair-2026-09-29` contains the benchmark
  process-identity fix and v3/retained-grader CI repairs. See `CI_V3.md` for test and lint scope.
  Do not promote until CI and the exact-revision hardware bar finish; previous results below
  do not certify this candidate. The live ioeventfd accelerator remains a subsequent experiment.
- **2026-09-29 Paguro retirement:** the owner canceled the retention requirement. Source and
  unique artifacts were re-audited and backed up in Paguro commits `bb8cf91` and `17a51b9` on
  `recovery/resume-2026-09-28`; then instance `53076605` was destroyed. Vast inventory now contains
  only `53004208` (RTX 3060, about $0.1661/hour) for Kayfabe checks. The Windows VM is no longer
  running; its disposable images were not treated as irreplaceable work.
- **2026-09-29 integration verified:** `codex/recovered-integration-2026-09-29` combines the
  published allowlist baseline with Claude `826ef957` (merge `4e6e1fbf`) and recovered Turing
  code `576f5bb0` (merge `cd80712a`). Display allocation/free observation is moved to successful
  object application; rejected operations cannot replace/release live display channels.
  **Exact `d883d0eb`: 1683/0 crate tests, gates 9/9, KF3_RC=0, fresh FG_RC=0, thin 30/30**,
  terminal EXIT 2026-09-29 13:33:52 UTC on retained RTX 3060 `53004208`.
  All 96 evidence files are recovered and hash-checked in `traces/recovered_integration_20260929/`.
  **Promoted to master/v3 as `5c639f54`**, documentation/evidence only after the tested revision.
  Historical TU116 results do not certify the new candidate on Turing hardware.
- **Separate benchmark candidate:** `2676902f` on `codex/benchmark-pid-2026-09-29` binds LLM
  perf counters to the launched QEMU PID/starttime, with GPU-free identity tests. GitHub-backed,
  not included in this merge bar or promotion. No production doorbell fast path or b3 module yet.
- **Display-on smoke at the same code:** guest boot and `nvidia-smi` pass, and the display model
  accepts a core channel. NVKMS then stalls on `0xc67d:0` progress; no DRM node/connector.
  This is incomplete display, not an M1 pass. Evidence is hash-verified and preserved in
  `traces/recovered_display_20260929/`. Next display work remains engine/worker/notifiers.
- **Historical; superseded by retirement above. Paguro recovered again, 2026-09-29:** at about 18:19 UTC, `53076605` reported stopped/exited,
  `GPU error, unable to start instance`, and direct SSH refused. One start retry restored the host.
  The verified Windows launcher reused the existing overlay/firmware/TPM state; QEMU PID 1564
  started at 18:22 UTC and guest SSH returned Windows 10.0.26200.6584. Retained for Paguro; no reset.
- **2026-09-29 checkpoint:** terminal network access is restored; GitHub, Vast and both boxes
  are reachable. The final run at **`61c49f14` passed: 1653/0 tests, gates 9/9, KF3_RC=0,
  fresh FG_RC=0, thin 30/30**, terminal EXIT on 2026-09-28 at 18:44:21 UTC. Evidence is now
  recovered locally in `traces/capability_535_545_audit_20260928/final-run/`. Changes through the
  published `9c3d87fd` are documentation/evidence only. **Promoted to master/v3 as `7c5b2a13`**; the
  earlier sandbox-blocked checkpoint is historical, not a failed test result.

- **2026-09-28 owner decisions:** 535/545 capability extension approved, subject to the independent
  audit and exact-revision merge bar; **b3 patched host nvidia-uvm selected, full native host CUDA
  coexistence required**; doorbells: **non-nested baseline and host-side/ioeventfd work first, guest
  helper afterward**, optional modified guest driver deferred. See §3 and `OWNER_RULINGS.md`.
- **Recovery is preserved, not implicitly merged:** source/evidence at
  [`recovery/resume-2026-09-28`](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md),
  recovered Turing code at `recovery/vast-tuwork-2026-09-28` (`2825c42f`, code tree `576f5bb0`).
  The Claude head `826ef957` has 30 commits beyond the previous baseline, integrated and verified
  in the exact combined candidate above.
  Do not use those historical results to certify a new candidate. The fresh allowlist candidate
  starts from the published baseline, so it does not silently promote the other pending changes.
- **Physical baseline access:** read-only SSH to `172.22.1.20` reports `Network is unreachable`
  from this workspace on 2026-09-28. Prepare the protocol locally; do not label a Vast KVM VM as a
  non-nested host. No physical-host session or display was changed. The owner subsequently
  requested the nested Vast ioeventfd experiment; label its results as nested, not physical-host.
- **Allowlist audit complete:** full 550–610 before/after policy comparison plus compiler-derived
  checks for all 16 admitted controls from the previously unchecked shared groups. Both the
  pre-change golden and the two header fixtures were independently regenerated on the trusted
  development host with byte-identical results. Final merge bar at `61c49f14` passed;
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
- **Historical box inventory, superseded by retirement above (2026-09-29):** `53080587` (GTX 1660 SUPER) was retired after
  rechecking all 493 saved evidence hashes and its recovered source tree. Retained: `53004208`
  (RTX 3060, verification/next Kayfabe tests) and `53076605` (Paguro Windows). No new rentals.
  Both retained instances are running, and Paguro's Windows QEMU remains live. Combined listed
  rate is approximately $0.5203/hour before additional fees. Preserve evidence before retirement.
- **Historical pause notes below are superseded by this resumption.** mc21 was promoted to
  `8ab92bf4`; older "awaiting promotion", "all boxes being destroyed", and open-choice lines below
  describe earlier points in the campaign, not current actions.
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
- **Owner decisions resolved 2026-09-28**: §3. The cloud-only execd shared-key question remains
  separate; direct SSH without agent forwarding avoids it and does not block this work.

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
| Families | GA10x (GA106/GA104/GA102) 30/30; Ada AD106 30/30; Blackwell GB203 (RTX 5080) 30/30 + CUDA ladder; floor-swept RTX 3060 Ti 30/30. TU116 (GTX 1660 SUPER) at master `1915bd71` (code `3f67ed95`, 2026-09-30): tests 1742/0, gates 9/9, bare 30/30, thin 30/30, CUDA ladder host 4/4 + guest 4/4 (`traces/v3_turing_master/`). GA100, Hopper, GB10x remain source-derived only; GA100/GB10B refused by name | [Turing recovery evidence](https://github.com/reindertpelsma/kayfabe/blob/recovery/resume-2026-09-28/docs/RESUME_2026-09-28.md), `design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_FLOORSWEPT_GR.md` |
| Multi-GPU | distinct host GPUs in one VM work (8×3060 box); per-card BAR1 budget refused at realize | `design/V3_MULTI_GPU_AUDIT.md` |
| CUDA apps | 60/65 at `4c48ca0c`, 6/6 stream probes, 100/100 processes; four UVM-demand-paging failures plus CDP child-launch failure. Not rerun at the latest integration revision | `design/V3_APP_MATRIX.md` §R3 |
| Graphics / video | nvkvm-pv's headless graphics set + 15 more items: **38/38** on an RTX 3070 (31 byte-identical to bare metal; OFA optical flow advertised); NVENC/NVDEC byte-exact. Per-call GPU waits are slow on nested boxes (`glFinish` 62 vs 9 µs) | `design/V3_GFX_TESTSET.md` (display-phase list §7), `V3_HEADLESS_GRAPHICS.md`, `V3_VIDEO_ENGINES.md` |
| Memory plane | pooled walker capacity (no per-space 16k-run wall); batched host maps; big-PTE slot ownership; guest PTE read-only/volatile carried, PRIV leaves withheld from user twins; **every host map snoops the CPU cache** (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE` — without it a CE read stale DRAM on bare-metal hosts; nested VM boxes hid it) | `design/V3_BUILD.md`, `V3_BATCHED_MAP.md`, `traces/v3_adasys/FINDING.txt` |
| Refusals | audited host-vs-guest: forged completions removed (MC_SERVICE_INTERRUPTS, sysmembar flush); the rest classified | `design/V3_REFUSAL_AUDIT.md` |
| Driver matrix | ★ (2026-09-27, the `v3-drivers` code on master since `5018bb57`) CUDA ladder 4/4 for guests 580.159.04 / 580.105.08 / 590.48.01 / 595.84 / 575.57.08 / 610.57.04; hosts 575.57.08 / 580.95.05 / 580.65.06 gates 9/9, thin 30/30, ladder 4/4, mixed pairs 4/4 — measured on `v3-drivers` heads before the merge (grid §6.0, derived from `traces/driver_matrix/walk/`); 535/545 capability rows approved, independently audited and merged (§3.1); no 535/545 end-to-end application claim. Earlier: 29 ogkm tags measured into generated tables; guest 580.x works end to end; ≤575 guests pass RM init (fn 54/79 carried); host 575.57.08 gates 9/9 | `design/V3_DRIVER_MATRIX.md` |
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

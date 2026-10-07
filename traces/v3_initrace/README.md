# traces/v3_initrace — evidence for the adapter-init flake and the WPR2 retry fix

**STATUS: LIVE, 2026-09-26 (branch `v3-initrace`).** Box: vast `52792102` — RTX 3090 (GA102
`0x2204`), Xeon E5-2673 v4 (10 cores, nested KVM), machine `97012` (the same machine as the
driver-matrix box `52746206`), host driver 580.159.04 open. Every file is text, xz-compressed; the
`summaries/` lines are one per run (`tag bin=<kf3 revision> … verdict … counts`).

## Revisions

| rev | what it is |
|---|---|
| `02b27c2a` | master at the start (no instrumentation) |
| `626afea1` / `30a67f0b` | probe + cycle mode + `KF3_INJECT_STALE_USERD`, **no fix** (pre-/post-rebase) |
| `555e56e6` / `dcde8b34` | + the USERD fix (+ the WPR2 fix at `dcde8b34`) |
| `7f271349` | the final code, rebased on master `59cc98a9` — every `f_*` run, `tests_7f271349`, `v3_gates_7f271349`, `f_suite_suite.out` |
| `*-dirty` | the same head with the FRTS-command read cut out (the (B) control) |
| `e9e1e328` | ABORTED — a shared cargo target dir linked a stale `kf-arch`; its logs are kept only as the record of that |

## (A) the adapter-init flake — a Translated channel born over a stale USERD

- `summaries/initrace_batch1.summary`: before — `b1_before_nat` / `b1_master_nat` 0/300 natural
  failures each, but 299/300 opens born over `GP_PUT=GP_GET=2` (`tok2_stale_at_birth=299`,
  `forwarded=4098` in `logs/fast_b1_before_nat_qemu.log`); `b1_before_inj` **20/20 fail** with the
  stale `1` injected; after (`555e56e6`) 0/300 natural, 0/300 injected.
- `logs/fast_inj1_*` — the reproduced failure: the guest's `memmgrMemSet … NV_ERR_TIMEOUT`,
  `ce_utils.c:349`, `RmInitAdapter failed (0x25:0x65)`; kf3's `RETIRED, forwarded=1
  submissions=0 … gp_get=Some(1)` and the probe's dump (`releases=[]`: the one fence was the NOP
  retire; `CE2:wakes=2,raised=0`). `logs/fast_inj2_*` — the same injection, fixed: PASS.
- `summaries/initrace_final.summary` `f_A_nat` / `f_A_inj` — at `7f271349`: 0/300 and 0/300,
  300/300 `forwarded=2 submissions=2`.
- `summaries/initrace_batch3.summary` — a **580.65.06 guest** (`fastguest-580.65.06`): 150 cycles
  and 6 × (`--timer`, `--concurrency`, `--uvm-mean`) before and after — no failure either way;
  before shows the stale slot (`149 forwarded=4098`), after `150 forwarded=2`.

## (B) WPR2 after a failed GSP boot

- `summaries/initrace_final.summary` `f_B_*` (and `initrace_b4.summary` at `dcde8b34`): a 16 MiB
  WPR-end margin through RM's own `RmGspWprEndMargin` regkey (fast guest `KF_NVREG`), with and
  without one injected FWSEC failure (`KF3_INJECT_FWSEC_FAIL=1`). Control: *"WPR2 initialized at an
  unexpected location"*, 5/5 later opens dead; fixed: WPR2 served at `0x1fee00000` (16 MiB below
  the zero-margin `0x1ffe00000`), the retry boots, 20/20 later opens pass.
- `logs/fast_b2_B_*` — the same injection WITHOUT the regkey: both binaries fail in the guest's
  own RM (`kgspAllocateScrubberUcodeImage … NOT_SUPPORTED`, `RmInitAdapter 0x62:0x56`): RM's full
  retry margin needs the secure scrubber, which only AD10x ships.

## Verification at `7f271349`

`tests_7f271349.log` (1598 passed / 0 failed), `v3_gates_7f271349.log` (9/9),
`f_suite_suite.out` (30/30, `KF_DEVICE=kf3 fast_suite.sh f_suite 180`).

## The coordinator's 100% reproducer — a 570.148.08 guest re-initialising (`570/`)

`summaries/initrace_570dr.summary`: base `origin/v3-drivers` `607f290f` (the driver-matrix head),
**before** = + the probe/log commits (`88438930`), **after** = + the USERD fix (`f5843151`); the fat
guest `guest-570.148.08.qcow2`, `cuda_ladder.sh guest`.
- before, `570/run_cl_l570dr_88438930_cup2_1_qemu.log`: token `0x802` (CeUtils) born over `(0,0)`
  then TWICE over `(Some(1), Some(1))`; the first life retires `forwarded=1 submissions=1`, both
  later ones `forwarded=1 submissions=0` — cup2 FAIL.
- after, `570/run_cl_l570dr_f5843151_cup{2,3,8}_1_*`: the re-init's CeUtils retires
  `forwarded=1 submissions=1` (releases LANDED in the probe dump); the ladder then stops at a
  DIFFERENT wall — nvidia-uvm 570's first channel (token `0x803`) `DEAD: ring: Read … 0x121010000
  not placed by us` → `REFUSED-AND-POISONED` (refused by name, never retired); cup2/cup3/cup8 FAIL
  there, cup8bench not run.
- `summaries/initrace_570.summary`, `570/run_cl_l570_59cc98a9_cup2_1_*`, `570/fast_t570_*`: on
  master `59cc98a9` (fat guest) and in the thin guest on either binary, a 570.148.08 guest oopses in
  its FIRST init (`memmgrMemCopyWithTransferType`, NULL dereference) — not this defect.

## The second variant (self-test data mismatch) — open

`summaries/initrace_hunt.summary` — the 580.65.06 guest's `--uvm-mean` / `--concurrency` /
`--timer` arms under `KF3_COMPLETION_PROBE` (lines `rc=2` from `h_*_1…110` are an aborted loop
that had no binary; the valid rounds follow). ⊘ The probe's `DID NOT LAND` on UVM's token is a
read too early (a sysmem release trailing our vidmem fence), and `DST != SRC` fires when a later
launch in the same fence rewrote the destination — neither is a finding by itself.

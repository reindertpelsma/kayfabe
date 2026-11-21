STATUS: IN PROGRESS, 2026-11-21 (hypotheses written BEFORE the first run; results are appended below, never edited above this line).

# N1 control: do enscape-01 `06_SanMiguelFullUltraHQ` and solidworks-08 `08_NASACrawlerShadedEdges` still stall with a larger kf3 framebuffer?

Finding N1 is from standalone wave 1b (`traces/standalone_wave1b_windows_20261121/README.md`, PR #558): on an RTX 3090 (GA102) / Xeon W-2170B box, kf3 `fb-mb=8192`,
revision `9b3d16519`, the two subtests above did not complete inside the viewset budget (6300 s and 3000 s), and after solidworks-08 the guest did not stop gracefully.
solidworks-08 completed under kayfabe on 2026-11-05 on an RTX 4090 (rev `6a0683af`, offline guest). unreal_engine-01 was not run (a leg-deadline config error).

## Hypotheses and falsifiers (stated before any run)

- **H_vram (a CONFIG matter):** the stall is the 8 GiB framebuffer kf3 exposes to the guest being too small for these two subtests.
  - SUPPORTED if, on the same box type (RTX 3090, same revision, same stage) with `fb-mb=16384`, both subtests complete (status line `Completed`).
  - FALSIFIED if either of the two subtests still does not complete within its budget at `fb-mb=16384` (a stall is: no `Completed` for the subtest after the budget, where its High/non-edges twin
    took 8.1 min (enscape-01 05) and 68 s (solidworks-08 07)). A single row that still stalls falsifies "the framebuffer size is the cause" for that row; the other row is reported separately.
  - Knob: `WINVM_KF3_FB_MB` (win_vm.sh, `docs/howto/cli-reference.md` property `fb-mb`); no kayfabe code is changed. It is exported before the Windows gate and the SPEC stage so that every boot of the run
    uses it (the gate's own boots too). The guest-visible size is read back from the gate's `nvidia_smi_q.txt` (FB total), not assumed.
- **H_die:** a GA102 (3090) versus AD102 (4090) effect, or a kayfabe gap specific to GA102. Only a native control on the same die separates them: native Windows (windows-on-vast) on the same RTX 3090 type running the same two
  viewsets. It is run ONLY IF H_vram is falsified (plan: cheapest first, stop at the first conclusive answer, budget 3.0 USD and 4.5 h in all).
  - Native completes both subtests in a few minutes and kayfabe stalls at 16384: a kayfabe gap on GA102 (not a framebuffer matter).
  - Native also stalls or takes more than the kayfabe budget: a die/app limitation, not a kayfabe gap.

## Plan and named deviations (planned before the run)

- Same sha as wave 1b (`9b3d16519`) so that the framebuffer is the only changed variable (plus the box itself). Provision and Windows gate through the payload
  (`vast_payload.py --only provision --only windows_gate`), then the SPEC stage by hand through its wrapper `scripts/ci/vast_stage_specviewperf.sh`.
- D1: the payload strips `KF_SPEC_TABLE` from its stage command (a narrowed run can never certify a lane), so the three viewsets cannot be selected through the payload. The stage is therefore driven by hand with
  `KF_SPEC_TABLE` (a table shaped like `lanes.json`, lane name `pcn1`); every verdict line then ends with `NARROWED`. This is the documented proof-run knob.
- D2: budgets in that table are shorter than `lanes.json` (4.5 h wall clock does not fit a 6300 s stall): the table is recorded in this directory. Row order: solidworks-08, enscape-01, unreal_engine-01
  (the stage's time guard decides what fits; a viewset it refuses is reported NOT_RUN TIME, not a verdict).
- Input: the SPEC bundle only through a presigned GET minted on the coordinator (read-only token, 12 h TTL at most, URL file deleted from the box after the copy). No token, key, e-mail or licence identifier on the box.
- Evidence: text only, GPU UUIDs/IPs/URL parameters redacted.

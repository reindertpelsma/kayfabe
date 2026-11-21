STATUS: LIVE, 2026-11-21 (final). MEASURED on one rented Vast box (nested KVM guest; text evidence only). The hypotheses, falsifiers and the plan below were committed BEFORE the first run (git history of this file: first commit); only this line and the sections from "Results" on were added afterwards.

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

## Operator decision rule for enscape-01 `06` (written 2026-11-21 21:58Z, BEFORE its outcome is known; measured so far in this run: solidworks-08 PASS 8/8 at fb-mb=16384, see Results)

The 4.5 h wall clock does not leave room for both the full 5400 s budget of enscape-01 and unreal_engine-01 (needs about 45 min incl. boot and settle). Rule: if `06_SanMiguelFullUltraHQ`
(started 21:42:42Z; 05, its High twin, took 6.4 min in this run; UltraHQ is about 2.1 times its High twin in 01/02 and 03/04; native 4090: 8 min) has not completed by 22:10Z (27.5 min, about 3.4 times the native wall),
I kill the SPECviewperf CLI inside the guest (operator kill, recorded) so that the stage goes on to unreal_engine-01 in the same guest. enscape-01 is then reported as a stall by operator kill (not as the stage's own TIMEOUT), with that wait stated.
If `06` completes before 22:10Z the viewset runs on to its normal end. Wave 1b's 8192 run waited 66 min without completion.


# Results

## Boxes, revision, what ran
- ONE box, label `kf-accept-n1-185503`, instance 61844366, destroyed by id at 22:48Z (absence verified: `vastai show instances` listed no instance of the account at 22:48Z). Same offer as the wave 1b SPEC box (offer 61777894, RTX 3090 GA102, 24576 MiB,
  Xeon W-2170B VM, 13 vCPU, 98 GB, Quebec, KVM image, 300 GB disk request: `df -h /` showed 291 G right after boot), host driver 580.159.04, same kayfabe sha as wave 1b: `9b3d165192e03b5cd27a7eca1a0f7f494cbc058a`. No kayfabe code changed, no test relaxed.
- Spend: 18:55Z to 22:48Z = 3.88 h at 0.441 USD/h (the offer's `dph_total`, which includes storage) = about 1.71 USD (ESTIMATE: rate times time; the account was not read afterwards). Wall clock of the whole task: about 4.1 h of 4.5 h.
- provision + windows_gate through the payload (`vast_payload.py --instruction gate/instruction.json --only provision --only windows_gate`, `WINVM_KF3_FB_MB=16384` in its environment): provision 720 s (`READY driver=580.159.04`), windows_gate PASS 3142 s,
  `KF_FINAL_SUMMARY verdict=PASS stages=2` (`gate/gate_facts.txt`). The guest-visible framebuffer was read back, not assumed: `nvidia-smi -q` in the gate's guest: `FB Memory Usage Total : 16384 MiB` (`gate/gate_facts.txt`); every QEMU command line of this run carries `fb-mb=16384`
  (`gate/qemu_fb_property.txt`).
- the SPEC stage by hand through `scripts/ci/vast_stage_specviewperf.sh` with `KF_SPEC_TABLE=lanes_n1.json KF_SPEC_LANE=pcn1 KF_SPEC_NET=online WINVM_KF3_FB_MB=16384` (deviation D1). SPEC bundle: presigned download-only URL (6 h TTL, minted on the coordinator with the release kit's `mint_urls.py`, read-only token),
  file at `/run/kf-ci/download-urls.json` mode 0600, shredded on the box at 20:31Z right after the copy and removed on the coordinator; the stage computed sha256 `ffe0e505...14d5a` (equal to the git pin, `spec/trust.jsonl`). Copy of the three viewsets (33.5 GB of tree out of the 42 GB archive) 20:09Z to 20:29Z, verified against the manifest.
  No token, key, e-mail or licence identifier was put on the box. The stage ended `SPECVIEWPERF_STAGE_FAIL NARROWED`, `SPEC_STAGE_COUNTS pass=1 fail=1 not_run=1 of=3`, guest stopped gracefully in 26 s, host reference green before and after (`hostref_*`: cup8 PASS, Xid delta 0).

## Per-row table (guest walls from `SPECRUN_END`; native = `native_s` of `lanes.json`, a 4090, so a ratio mixes kayfabe overhead with the slower die and is not an overhead figure)
| viewset | verdict | subtests | wall in guest | native (4090) | last subtest reached | wave 1b at fb-mb=8192 |
|---|---|---|---|---|---|---|
| solidworks-08 | **PASS** (reason `-`, 0 resets: `display4101=0 nvlddmkm153=0 livekernelevent193=0`) | 8/8 | 1823 s | 989 s (ratio 1.84) | `08_NASACrawlerShadedEdges` Completed (started 20:55:43Z, completed 21:07:46Z: 723 s; native 4090: 363 s) | `08` not completed in more than 32 min, budget 3000 s hit, guest did not stop |
| enscape-01 | **FAIL, stall in `06`** (stage reason `SUBTEST`, 5/10; ended by my operator kill, see D3, NOT by the stage's TIMEOUT) | 5/10 | `06` ran 27.6 min (21:42:42Z to the kill at 22:10:16Z) without completing; viewset 61 min | 2042 s (native `06`: 8.0 min) | `06_SanMiguelFullUltraHQ` Started, never Completed | `06` not completed in 66 min, budget 6300 s hit |
| unreal_engine-01 | **NOT_RUN, reason TIME** (the stage's time guard cut it at its budget of 1756 s, deadline 22:55Z) | 2/8 Completed (03, 04); **01 and 02 `Failed`**; 05 running at the cut | 1756 s (cut) | 1236 s | `05_CitySampleLitHitLighting` Started | not run in wave 1b |

Subtest minutes in this run (enscape-01 01..05): 6.0, 12.6, 2.3, 5.0, 6.4 (wave 1b: 6.7, 14.1, 2.5, 5.3, 8.1). solidworks-08 `05_JetEnginesShadedEdges`: 597 s (wave 1b: 597 s; native 4090: 313 s), so the slow `05` is the same at 8192 and 16384 and is not the stall.
unreal_engine-01: `01_VotALitDetailTracing` Failed after 11.4 min (native 3.6 min), `02_VotALitGlobalTracing` Failed after 11.0 min (native 1.3 min), `03` Completed in 1.5 min, `04` Completed in 1.6 min. This is the first time unreal_engine-01 ran under kayfabe in the record
(2026-11-05: not run for lack of time; wave 1b: my config error), so the two Failed subtests are a new observation (finding N3 below), not a regression.

## Host evidence for the stall (MEASURED, `host_va_lines.txt`, `spec/host_samples_and_status.txt`)
- enscape-01 `06`: at 21:47:37Z (4 min 55 s after `06` started) the host kernel logged `NVRM: virtmemAllocResources: VA Space alloc failed! Status Code: 0x51 Size: 0x90000 RangeLo: 0xb0000, RangeHi: 0x13ffff, pageSzLockMask: 0x11000`, preceded 13 s earlier by `kf3-vamgr` PAT unmap lines.
  **The same line (same status, size and range) was logged in wave 1b at fb-mb=8192**, 8 min 08 s into `06`, again after PAT lines 12 s earlier (`traces/standalone_wave1b_windows_20261121/s/workspace/kernel.log`, 15:48:51Z). No Xid in either run.
- the guest was not frozen: `EnscapeClientBench.exe` and the SPEC CLI stayed alive and used CPU (7 s of CPU in a 38 s window for the benchmark process at 21:57Z), the licence stayed `Licensed`, host GPU utilisation read 0 % and 17.4 GB used (16 GiB framebuffer plus host RM).
- unreal_engine-01: three more lines of the same family: 22:17:09Z `Size: 0x110000 RangeLo: 0x20000 RangeHi: 0x12ffff`, 22:18:18Z and 22:29:26Z `Size: 0x80000 RangeLo: 0xb0000 RangeHi: 0x12ffff` (the first two at the start of `01`, the third during `02`).
- solidworks-08: NO such line in its window (and none in wave 1b's window for it).
- Native references already in git/private notes (MEASURED, 4090, not this die): peak host GPU memory of a 5 s sampler: enscape-01 5157 MiB, unreal_engine-01 5754 MiB (`traces/specviewperf_native_20261107/box1/*/smi.csv` on branch `claude/specnative-20261107`), solidworks-08 2238 MiB
  (private `specnative2` evidence, `smi.csv`); the native `06` took 8.0 min and the native `08` 6.0 min.

## Conclusion (per hypothesis; classification is one of the four CLAUDE.md classes per row)
**H_vram is FALSIFIED as the explanation of N1 (the pre-registered falsifier fired for enscape-01), and SUPPORTED, with a named confound, for solidworks-08.**
- **enscape-01 `06_SanMiguelFullUltraHQ`: still stalls with the guest seeing 16384 MiB** (27.6 min against 8 min native, 3.4 times the native wall; wave 1b waited 66 min at 8192). The framebuffer size is not the cause. The same host RM `VA Space alloc failed 0x51` line appears in the 06 window at 8192 and at 16384.
  Classification: **kayfabe gap, suspected, NOT confirmed by a control on this die** (the inference: identical host RM refusal in both configurations, native 4090 completes the subtest in 8 min using 5.2 GiB, which is below 8 GiB). H_die (GA102 versus AD102) is NOT tested: it needs native Windows on the same RTX 3090, see the cost below.
  The nearest documented mechanism is the host RM's VA reservation alignment (`docs/design/V3_HELD_HOLE_HANDOFF_20261010.md`, `docs/design/V3_BATCHED_MAP.md`); that its geometry (a low window `0x20000..0x13ffff`, 576 KiB to 1.1 MiB requests) is the same defect is NOT established.
- **solidworks-08 `08_NASACrawlerShadedEdges`: completes at fb-mb=16384** (723 s, row PASS 8/8, 0 resets, 1.84 times native). It did not complete at 8192 in wave 1b. By the pre-registered criterion this SUPPORTS H_vram for this row. **The confound**: one run each side, on different boxes and days, no 8192 control in this run, and the native peak of 2238 MiB makes a
  framebuffer-demand mechanism implausible (INFERRED), so a non-reproducible stall of wave 1b is not excluded. Classification: **open, between config error (fb-mb) and a non-reproduced stall**; the missing control is one boot: this stage with `WINVM_KF3_FB_MB=8192` for solidworks-08 only on the same box type.
- **unreal_engine-01** was run (it was a config error not to in wave 1b): cut by the time guard (NOT_RUN TIME, a config matter of my time box), but 2 of its first 8 subtests `Failed` with host VA lines in the window (N3).

## Proposal for the owner (NOT changed in this PR)
- The SPEC stage's default `WINVM_KF3_FB_MB` is win_vm.sh's 8192 (the stage only lowers it on cards of 10 GB or less). On 24 GB cards, 16384 left the stage healthy (host reference green, no Xid, 17.4 GB of 24.5 GB used) and is the only setting under which solidworks-08 `08` completed; proposal: 16384 for cards of 24 GB or more,
  8192 below, after the solidworks-08 control at 8192. It does **not** fix enscape-01 (nor, probably, unreal_engine-01), so it must not be recorded as the N1 fix.
- The lead for enscape-01 and unreal_engine-01 is the host VA reservation failure in the ray-tracing subtests (both viewsets require `raytracing`); a developer item, not a SPEC-stage knob.
- Cost of H_die (not run): native Windows (windows-on-vast, `reindertpelsma/windows-on-vast`, ISC) on a 3090 offer, enscape-01 only: rental, Windows install, driver and CUDA about 35 to 40 min, SPEC archive streamed through a filter (only the 2.2 GB enscape-01 tree is kept, 42 GB read at about 45 MB/s: about 16 min), virtual display setup about 10 min,
  the viewset about 35 to 45 min on a 3090 (native 4090: 34 min) = about 2.2 to 2.5 h and about 0.9 to 1.1 USD at 0.35 to 0.45 USD/h, plus a risk of a dead-on-arrival rental (3 of 8 in the 2026-11-07 native run). It would have been run in parallel to this box only by renting before the stall was known; after the stall (22:10Z) it needed more than the 0.6 h left of the 4.5 h wall clock.
  Spend would then have been about 2.7 USD of the 3.0 USD budget.

## Findings
- N1a (enscape-01 `06`): stalls at 8192 and 16384, same host RM VA refusal in the window. Kayfabe gap suspected; H_die open; next step a native Windows run of enscape-01 on an RTX 3090.
- N1b (solidworks-08 `08`): completes at 16384, stalled at 8192 in wave 1b; one more run at 8192 on the same box type separates config from flake.
- N3 (new): unreal_engine-01 `01_VotALitDetailTracing` and `02_VotALitGlobalTracing` (ray tracing) `Failed -- check logs for errors` under kayfabe (RTX 3090, 16384 MiB), `03` and `04` Completed; host `VA Space alloc failed` lines in the window; no native failure of these subtests on a 4090 (native completed 8/8). Not classified: no control on this die; falsifier: the same subtests also fail natively on a 3090.
- Harness note H1: killing the SPEC CLI inside the guest with `spec_guest.ps1 -Action kill` also stops `spec_run.ps1` (the same scheduled task), so no `END.txt` is ever written and the stage's poll waits until the viewset budget. A real-hang path other than the budget (for example an operator kill) needs `spec_run.ps1` to survive the kill or the stage to treat a dead CLI as ended. Not fixed here.

## Named deviations
- D1: the stage was driven by hand with `KF_SPEC_TABLE` (the payload strips it from its stage command); every verdict line therefore ends with NARROWED. Table: `lanes_n1.json` (lane `pcn1`: solidworks-08, enscape-01, unreal_engine-01, in that order).
- D2: budgets shorter than `lanes.json` (solidworks-08 2700 s, enscape-01 5400 s, unreal_engine-01 3600 s) and the stage deadline 22:55Z, set by me so that teardown comes before the 4.5 h limit. The unreal_engine-01 budget was cut to 1756 s by the time guard; its NOT_RUN TIME is my time box, not a verdict on the viewset.
- D3 (operator intervention, rule written before the outcome, section above): at 22:10:16Z (27.6 min into `06`) I ran `spec_guest.ps1 -Action kill` in the guest so that the stage could go on to unreal_engine-01 inside the 4.5 h. The kill also ended `spec_run.ps1`, which never wrote `END.txt`; at about 22:14Z I wrote a one-line `END.txt`
  (`SPECRUN_END ... rc=-1 ... secs=0`, the `end=` time is my write time, not the kill time; `spec/operator_actions.txt`) so the stage's poll saw the end. The row was then graded by the stage's own code (`reason=SUBTEST`, 5/10), not as TIMEOUT. The wait of 27.6 min is shorter than wave 1b's 66 min; a longer wait would have cost unreal_engine-01.
- D4: during the run I read the CLI's status lines (`Started`/`Completed`/`Failed` only, with `findstr`/`grep` filters) and the process list of the guest over the stage's own ssh path, and host `nvidia-smi` samples (`spec/host_samples_and_status.txt`). No score was opened.
- D5: same sha and offer as wave 1b, but a new box on a new day; the 8192 side of every comparison above is wave 1b's record, not a control of this run.

## Scrubbing
GPU UUIDs are replaced by the all-zero UUID, URLs, IPv4 addresses and e-mail-shaped strings by script and re-checked by search; no image, screenshot, archive, URL file, QEMU command line (only the `fb-mb` property) or score is in this directory. The SPEC vendor result directory stayed in the guest and died with the box.

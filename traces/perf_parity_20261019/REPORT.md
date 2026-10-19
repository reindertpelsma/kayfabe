# P1 final report

1. **Lane and deliverable.** CODE READ: P1, branch `codex/perf-parity-20261019`, asked to measure Steel Nomad parity walls W1–W5, implement optional interrupt diagnostics and gated measurement experiments, and restore the owner's morning session. The final report commit is the branch tip; its exact local/origin hash is verified in the private handoff report. Immutable tested product source is `41a8beb07573847a932b76d48bc0dcd4383ea247`, built on mirror source `c2685412e159534b88097ea824d05fe963be83b1` (same mirror code as author binary `0d7ba74389cd85ac65826417ad53f5c0bcd1ef2f`). No merge, master update, TSG semantic change or production promotion occurred.

2. **Result (MEASURED / CODE READ; ten lines).**

- DONE: preregistration and W2/W3/W4 code reading; hypotheses/falsifiers precede observations in PREREG.md and its committed updates.
- DONE: default-off optional source/vector/arming/timestamp recorder and non-freezing relay counter checkpoints; production default features exclude their code (CODE READ, not an assembly benchmark).
- DONE: full-ring-only GET measurement flag, default off, with red-first software proof and Linux30/30; Windows qualification stopped on an unexplained automation failure.
- DONE: immutable mirror-plus-P1 binary, native9/9 plus gate10, diagnostic fast30/30, and full-GET fast30/30; individual fault attribution retained.
- DONE:20 clean Steel Nomad captures across mirror/control, diagnostics,20us and50us pacer arms, n5 each, plus77 pure tiny-probe invocations and three final DIRECT attribution traces.
- DONE: W2 observed median two ISR cycles/four actual irqfd writes per DIRECT window; source streams remain distinct; no redundant edge proved or removed.
- DONE: W3 found no equivalent current synchronous ioeventfd/irqfd shortcut; all79,145 observed CPU_INTR writes were nonzero.
- PARTIAL: pacer20/50 showed no new-content-fps gain beyond spread;50us increased ISR entries/frame. Keep pacing default0.
- NOT DONE: lazy-observe0 / PUT-cache0 arms, Windows GET performance, Heaven baseline/candidate fps, complete15-minute mixed gate, true host GPU-fetch decomposition, same-session VFIO parity.
- PARTIAL: unexplained Windows GET-mode launch/query timeouts remain; no candidate meets every morning promotion criterion. NOT DONE: production restoration and P1 lease release are blocked by the failed clean-shutdown check; actual06:30 state is recorded below.

3. **Tests, mutations and hardware evidence.** MEASURED: touched-crate release mirror baseline929 passed/0 failed/3 existing ignored; candidate default939/0/3; both optional features960/0/3. Earlier pre-mirror scope was833/0/3 and is archival, not the final comparator. Python harness17/17. rustfmt changed lines PASS. Complete Clippy baseline/candidate multisets match:89 inherited denied errors and364 warnings, added0/removed0; kf-chan default separately0 errors/220 warnings. Clippy exit101 remains explicit and accepted only under the coordinator's baseline-equality ruling; inherited diagnostics were neither fixed nor suppressed. See VALIDATION_A.json, CLIPPY.json and clippy_comparison.json.

MEASURED: separate EXPECTED RED commits precede trace, GET, counter/sink, broker/DDI parser, capture-freeze guard and Heaven-vsync fixes. Twenty-two restored-source mutants were killed by at least two tests each:

| Mutant | Failed tests | Evidence |
|---|---:|---|
| default_on | 2 | MUTATIONS.json |
| unbounded_tail | 2 | MUTATIONS.json |
| source_collapsed | 2 | MUTATIONS.json |
| snapshot_duplicated | 2 | MUTATIONS.json |
| eager | 4 | MUTATIONS.json |
| never_flush | 2 | MUTATIONS.json |
| missing_first_full_guard | 2 | MUTATIONS.json |
| stale_source_fullness | 2 | MUTATIONS.json |
| off_key_changed | 2 | MUTATIONS.json |
| stats_replayed | 2 | MUTATIONS.json |
| stats_freezes_trace | 2 | MUTATIONS.json |
| disabled_stats_active | 2 | MUTATIONS.json |
| acknowledges_failed_output | 2 | MUTATIONS.json |
| stats_uses_shared_stderr | 4 | MUTATIONS.json |
| duplicate_feedback | 2 | BROKER_MUTATIONS.json |
| accept_fallback_clock | 2 | BROKER_MUTATIONS.json |
| share_tid_zero_across_cpus | 2 | DDI_MUTATIONS.json |
| sort_same_tick_exits_first | 2 | DDI_MUTATIONS.json |
| ignore_freeze_request | 2 | TRACE_STATE_MUTATIONS.json |
| ignore_frozen_output | 2 | TRACE_STATE_MUTATIONS.json |
| missing_vsync_override | 2 | HEAVEN_PRESET_MUTATIONS.json |
| vsync_off | 2 | HEAVEN_PRESET_MUTATIONS.json |

MEASURED: native v3 gates9/9 plus gate10 PASS. Diagnostic-ON fast suite30/30; diagnostic+GET fast suite30/30 with actual flags verified in all30 processes. Each fast suite deliberately generated five Xid31 faults in defer-liveness/missing-page containment arms; exact PID/address/serial fault markers match, with no other new Xid. These intentional negative faults are not a claim of zero total host faults. Hardware evidence: NATIVE_GATES.txt, FAST_SUITE.txt, GET_FULL_GATE.json and private raw logs under `/data/codex-P1/`.

MEASURED: performance arms share the same persistent throwaway Windows disk and byte-identical cursor6 broker (`f4286897eaaa06b5267ab606c5f8d285f9e67d068b1cecddb96d5be764d7d252`). Control order is author mirror A3 / P1 diagnostic B5 / mirror A2, then pacer20 and pacer50; every configuration reboots. Later pacer arms are sequential, so residual drift remains a limitation. Their added setting is the only experimental variable relative to diagnostic ON, apart from per-arm dump filenames. Flags below are additions to the identical production profile; full actual flags, hashes and all per-run counters appear in RESULTS.json. NSI means `KF3_DIAG_NSI_TRACE=1`; interval is `KF3_PT_NSI_MIN_INTERVAL_US`; GET, lazy/cache overrides stay unset here.

| Arm | Product revision | NSI / interval us | n | Content fps median [min,max] | ISR entries/frame median [min,max] |
|---|---|---|---:|---|---|
| Mirror |0d7ba743|off /0|5|29.200 [28.773,29.884]|22.613 [22.034,23.639]|
| Diagnostic |41a8beb0|on /0|5|29.828 [29.281,30.115]|22.578 [22.447,22.670]|
| Pacer20 |41a8beb0|on /20|5|29.854 [29.584,30.351]|22.481 [22.006,23.694]|
| Pacer50 |41a8beb0|on /50|5|29.748 [28.988,30.153]|23.469 [23.084,24.588]|
| Full-ring GET Windows |41a8beb0|on /0 +GET1|0|unmeasured|unmeasured|
| Lazy-observe0 / PUT-cache0 |unrun|unrun|0|unmeasured|unmeasured|

MEASURED: **RESULTS.md contains every completed Steel Nomad run**, guest/app reference fps/score, new-content fps,1%/0.1% lows, >33/>50/>100ms gap counts/min, max gap and repeat rate; **MICRO_RUNS.md contains all77 independent pure probe runs** with rev/flags/iterations/median/p99/max; RESULTS.json has source identities, actual flags, raw archive hashes and remaining fields. Spread is min/max of independent run averages; no outlier is discarded. Controls include1.067s/1.083s content gaps, whose causes remain unverified. Some mirror guest/broker counts differ by at most0.359%, within the requested1–2% envelope. UI online validation is not asserted: pacer20 warning tooltip explicitly says not validated online; one occluded UI fps and one uncollected mirror score stay missing.

MEASURED: all accepted Windows performance arms have zero System4101, event153 reset records, application crashes and new host Xid through their cumulative health observations. GET Windows initially passed90s boot health, then interactive Steel Nomad launch and a read-only dispatch timed out: no benchmark data or cause established; host had0 new Xid. No further performance attempt followed. Earlier setup ROM failure, one new-meter-broker boot reset, incomplete terminal-marker probes and one rejected clock calibration remain retained in HARDWARE_PROGRESS.md; only specifically labelled unchanged quiet repeats that passed were admitted under the lane exception. No timeout, poll, duration or threshold was changed as a cure.

| Wall | Predeclared hypothesis / falsifier | Result and measured gain |
|---|---|---|
| W1 |Cross-TSG cost; falsified if mirror DIRECT/COMPUTE GPU leg does not fall below300/600us or Steel Nomad remains within spread.|MEASURED by TSG author: DIRECT840→268us, COMPUTE1486→233us, score2479→2876–2996. P1 consumed its binary and independently measured mirror tiny DIRECT252.4us and COMPUTE217.5us medians (n10 each), content29.200fps (n5). Author's exact GPU-leg comparison was not independently remeasured; W1 supported.|
| W2 |Redundant/late edges cause ISR cost; falsified if removing proven redundancy changes neither ISR/frame nor fps beyond spread.|MEASURED: three DIRECT traces have median2 cycles/14 register writes/4 actual irqfd writes, source medians2 GR0 and2 completion-fd offers. CODE READ: distinct subscriptions and ambiguous/coalescing HOST/FIFO/CE fd prevent proving duplicate physical completions. No edge removal; this falsifier remains untested. Pacer20/50 fps overlaps OFF; no demonstrated gain.50us ISR cost increases.|
| W3 |Mask/unmask writes might admit a synchronous cheaper equivalent; shortcut falsified by nonzero data or shadow/read/W1C dependence.|MEASURED:79,145/79,145 writes nonzero. CODE READ: current ioeventfd cannot apply arbitrary written mask/ack state synchronously; irqfd emits delivery but does not consume these BAR0 semantics. No equivalent implementation or gain established.|
| W4 |Lazy observe/GET sync costs matter; falsified by no beyond-spread change.|MEASURED: full-GET Linux30/30; Windows launch/query unresolved, n0. Lazy0/PUT0 unrun. No performance inference. Reported work/carried safety-net counters0 in accepted diagnostic runs, but CODE READ finds a pop-only GET-writeback branch not charged to work; counters do not exonerate all fallback progress.|
| W5 |Remaining GPU leg can be decomposed with ring/fetch/IRQ timestamps.|MEASURED: CPU-marker/IRQ traces overlap and sometimes yield negative differences; no true GPU-fetch timestamp exists. No GPU execution duration or gain inferred.|

4. **Owner decisions (ranked).**

1. Authorize the reviewed QMP abort of ONLY the failed P1-owned VM41009, then release P1's lease and restore production0452613b; the common STOP rule requires an explicit override of the clean-shutdown refusal. No candidate meets every promotion gate.
2. Investigate Windows GET-mode launch/query stall before any performance retry; retain flag default off. Read-only host health does not establish GPU correctness or a harness-only cause.
3. Keep pacer default0;20/50us have no measured beyond-spread fps benefit, and50us increases ISR/frame.
4. Preserve every armed source; first improve completion-source attribution and account for pop-only safety-net progress, then investigate exact remaining guest ISR work. No redundancy-removal patch is justified yet.
5. After correctness closes, complete lazy/cache arms and order-balanced Heaven/Steel Nomad controls, plus true channel-fetch timestamps; retest VFIO parity on the same disk/session.

5. **Could not verify.** MEASURED / CODE READ: full Windows GET correctness, runtime Heaven vsync/fps, complete15-minute mixed gate, M1's unrun extra CUDA/Vulkan/video/broker matrix, long-run stability, all physical edge completeness, cause of retained long gaps/reset/guest-dispatch failures, and a real fetch-to-interrupt GPU duration remain unknown. The optional trace is a bounded tail with nonblocking contention losses, explicitly counted; it cannot be a loss-free physical census. The diagnostic compilation defaults are reviewed, but production assembly equivalence is not measured. No parity achievement or production readiness claim follows.

6. **Final host and cleanup.** MEASURED at actual06:30 CEST (UTC observation `2026-10-19T04:30:20.494331+00:00`): **production is NOT running.** P1 test VM41009 remains alive, PID3718348, tested source41a8beb0, NSI1 +GET1, pacer0. Both ACPI and guest-agent shutdown failed. The helper's explicit refusal is retained in `get-full1-launch.log`; no bypass, force signal or lock break occurred. The lane instruction says, “If a step is blocked or refused by your own safety checks, STOP and report; do not reword or work around it.” P1 requested an explicit user override for reviewed QMP abort of only this owned VM; no response arrived at the morning snapshot.

MEASURED: the P1 launcher, owned broker, test QEMU and lease guardian/flock remain running. The global flock is still held by P1; therefore production restore and confirmation that nothing of P1 remains running are **NOT DONE**. Benchmark collectors have ended. Host GPU binding is `nvidia`, driver/utilization/memory `595.91.07, 2 %, 9007 MiB`, with0 new host Xid since GET boot. `/data/OWNER_MORNING.txt` was refreshed at the actual observation with this state, exact owned-VM recovery and guarded production0452613b commands. The prepared recovery preserves PID/start/executable/run identity and will not release a lease while a QEMU remains. The original owner launcher is unchanged. No production-ready morning state is claimed.

MEASURED: host cargo target is deleted. Dev cargo target `p1-target` remains561MiB because automatic approval review again rejected the authorized deletion command form; no alternate deletion was attempted. Guest-file cleanup and disposal of the throwaway overlay could not finish while its test VM remains alive. Do not reuse that overlay after its original backing disk changes. Immutable binaries, raw archived evidence and source ROM build assets intentionally remain under `/data/codex-P1/`. `MORNING_STATE.json` records the small public state; exact private PIDs/recovery commands and final verified origin head are in the private handoff report and morning note.

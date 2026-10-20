# C6 beta candidate topic ledger

STATUS: LIVE, updated 2026-10-20. CODE READ unless a row explicitly says otherwise.

CODE READ supersession: coordinator approved explicit `guest-os=windows|linux`
(default Linux), then separately approved the sweep-derived caps replacement.
This lifts the earlier guest-OS decision stop below. Only the scoped caps
module/data/helpers/tests are approved from the sweep; no full sweep merge.
MEASURED: new derived-caps EXPECTED RED tests are committed at `b1ea24127`;
implementation remains uncommitted after the mutation-wrapper stop documented
in [the report](README.md). No new product topic is accepted yet.
This is a BETA CANDIDATE assembly plan, not a release or hardware verdict.

MEASURED: the branch was cut from origin candidate-5
`f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`. The remote heads below were
queried read-only on 2026-10-19. No topic has been merged by C6 at this
checkpoint. **INCLUDE / queued** means selected for a separate integration
group after the Stage 2 guest-OS decision; it does not mean already present.
The explicit Stage 2 stop takes precedence over proceeding with that queue.

CODE READ: evidence from another lane is identified as a source report, not
as a C6 measurement. A passing test or benchmark on a source branch does not
certify a combined beta binary. All new hardware acceptance waits for the
coordinator's slot; the owner's VM is untouched.

## Production base — INCLUDE, already present

| Topic / branch and head | What is retained | Evidence and limit | Risk / decision |
|---|---|---|---|
| CODE READ: `integration/prod-candidate-5-20261018`, `f8f9d1a5f` | Candidate-4 plus gated hitch, LUT lifetime correction, boot-layer clean gate and reboot scrub, integration tests and source guards | Source report [final_verification.md](../candidate5_20261018/final_verification.md): MEASURED 3620 pass / 0 fail / 19 ignored, 22 crates; no candidate-5 hardware run in that report | INCLUDE, inherited production line. GPU-free acceptance is not Windows boot/aging acceptance. |
| CODE READ: `integration/prod-candidate-4-20261017`, `0452613b5` | Display pacing phases 1+2, tick fallback, bounded broker credits | Candidate-5 ancestry; [pacing design](../../docs/design/V3_DISPLAY_PACING_20261017.md), owner AK/AL | INCLUDE unchanged. Tick-source changes require a stop and written decision. |
| CODE READ: `integration/prod-candidate-3-20261017`, `525d7d7ad` | Broker cursor plane, true XOR, formats/import/hover/resize | Candidate-4 ancestry; owner AJ; candidate-5 source/blob and broker checks recorded in final verification | INCLUDE unchanged. No C6 compositor/hardware verification. |
| CODE READ: production lifecycle/lazy-observe series, last `userpump.rs` change `77da58c5a` | Default lifecycle, PUT cache and lazy observe; optional measurement controls retained | [lifecycle design](../../docs/design/V3_USER_RING_LIFECYCLE_20261016.md); production source is in candidate-5 | INCLUDE existing defaults. Do not import experimental lifecycle settings from newer launchers. |
| CODE READ: `claude/hitch-20261017`, `b233037d9`; gated source `codex/hitch-gates-20261018`, `560f9bce1`; merge `0cb1ed2fc` | Colour-path completion behind copy; step meters/event ring opt-in; broker stats clock gated by `--stats-line` | [diagnostic gates](../diagnostic_gates_20261018/README.md); source report covers failing-first/mutation gates. Supplied owner history: pre-merge hitch 56.9–57.6 Heaven average over 5 boots, not C6 measurements | INCLUDE merged gated form only. No claim that 1% lows or long gaps improved. |
| CODE READ: LUT correction `9ff3dc9e9`, merge `419d74b41` | Snapshot identity survives guest-life transitions | Source final verification reports two killing tests per LUT mutant | INCLUDE already merged; retain identity and reset interaction tests. |
| CODE READ: `claude/bootlayer-20261018`, `0dbcc8a14`, merge `f3f3351e3` | Clean boot-layer selection; unload zero; dirty-from-first-open reboot scrub | Owner AM; source report records integration and tests, hardware checklist still owed | INCLUDE already merged. Do not equate bus reset, TDR, unload and reboot. |

## Separately selected pure fixes — INCLUDE, queued

| Topic / branch and head | Argument for inclusion | Evidence and hardware status | Risk / integration boundary |
|---|---|---|---|
| CODE READ: F-2 subset of `codex/sweep-integrate-20261018` (`b6accbca2`); tests `08424cf11`, fix `6a32d4e8e` | Class-aware public upper-address widths in `kf-chan/src/grtables.rs` and `tmode.rs`; Ada/narrow and context-free validation remain 8-bit | Source `traces/sweep_integrate_20261018/F2_RESULT.md` reports 521→530 passes, 3 ignored, 9 focused tests, mutant kills 3/2/2/3; C6 has not rerun them | INCLUDE isolated two-file series, not the whole sweep branch. Inspect CE2 overlap in `tmode.rs` semantically and record it. Hardware in Stage 5. |
| CODE READ: CE2 `claude/ce2-kernel-ring-20261019`, `95e23df77` | Default CE decode class comes from the ring's owned host CE object, only for guest-kernel copy-engine work, when no guest SET_OBJECT exists; user/GR/video routes get none | C6 independently read the 8-file diff, class filter, caller and semaphore-width change. Separate EXPECTED RED commits `a8dc77dec`, `212b6db99`, `0b34d3eb8`; later source tests add second killers. No independent C6 execution yet | INCLUDE queued for independent test/mutation verification before integration acceptance. Windows TDR relevance is INFERRED, not a demonstrated cure. Birth/decoder boundary is sensitive; stop on any unresolved semantic conflict. |
| CODE READ: L5 `codex/bootlayer-followup-20261018`, `83071cb78`; product head `c81bd42d4` | Retire physical boot seed on sink-all; typed stale-life refusal cancels an unload restore instead of reseeding into a newer life | Source `traces/bootlayer_followup_20261018/VALIDATION.md`: 791→795 pass / 6 ignored, 2 killing tests per fix; hardware n=0 | INCLUDE two isolated failing-first/fix groups. Source base is bootlayer, not the completed candidate-5; retain current integration tests. Boot-layer scrub conflicts require a stop. Collector scripts are optional evidence tools, not part of boot. |
| CODE READ: nonblocking heartbeat subset in sweep head `b6accbca2`; green commits `32eebb3ab`, `4e0e0b645`, `d99c2367b`; red `16e80e802`, `39b11e805` | Removes blocking clock sampling from the drainer, uses mapped PTIMER and current-life fn72 clock metadata with bounded try-lock reads; keeps existing 500 ms eligibility | Source `PROTECTED_DECISIONS.md` records coordinator/owner approval of this exact nonblocking series; `HEARTBEAT_NONBLOCKING.md` traces the call path and source mutation receipts. Hardware UNRUN | INCLUDE this approved series only, after checking dependencies against candidate-5. Do not restore rejected blocking/clone/command-sequence imports. Doorbell/drainer or scrub conflict means STOP, not discretionary resolution. |

## Excluded or deferred topics

| Topic / branch and head | Evidence read | Decision / reason / what would change it |
|---|---|---|
| CODE READ: full sweep `codex/sweep-integrate-20261018`, `b6accbca2`; accepted Phase 1 product `24906bf33` | Source `RESULT.md`, `MATRIX.md`, `AD104_PATH_AUDIT.md`, `PROTECTED_DECISIONS.md`: source reports 3338/0/21 across 14 crates, 49 mutants with ≥2 kills, candidate-5 tests retained. Per-die constants/ZBC/caps, new public display layouts, FSP/reset, walker completion, diagnostics and harness changes span 406 files | DEFER full merge; include only isolated fixes above. AD104 strict native caps equality is RED by 100 words; AD104 hardware is UNRUN. AD102 Linux595 raw29/30 (timer failure also reproduced on candidate-4), Windows580 short hold PASS at `24906bf33`, Windows581 Code43. These mixed results do not establish owner-host parity or a complete family. No inference of new regression from the shared timer failure. Obtain exact-revision scope/slot acceptance before replacing the production paths. |
| CODE READ: sweep scrub fixture `94518f95b` | Approved tuple change adds `None` for the address-aware cache key; assertions and production scrub behavior retained | DEFER unless the corresponding surface-address change is imported. Do not apply an orphan fixture/type change to candidate-5 or count its compilation failure as a candidate-5 defect. |
| CODE READ: sweep family admission / per-die rows / diagnostic probes | Source AD104 path audit distinguishes authored fallbacks, measured exact die/branch rows and refused board values; `userrelay` family helper is a protected change | DEFER with the full sweep, except F-2. No copied native caps page, guessed neighbor row or automatic promotion of diagnostic probes. Separate reviewed imports and Stage 5 gates required. |
| CODE READ: TSG mirror `claude/tsg-mirror-20261018`, `c2685412e`; runtime `0d7ba7438` | Source report records legacy Linux30/30 and Windows isolated latency/Steel Nomad gains, but Linux explicit CUDA/Vulkan/video unrun | EXCLUDE. Behaviour-changing experimental grouping; mirror default ON in that source is not beta policy. Beta keeps `KF3_TSG_MIRROR` absent/OFF. |
| CODE READ: M1 `codex/tsg-mirror-gates-20261019`, `59df38e20`; M2 `codex/tsg-mirror-win-gates-20261019`, `24b122a33` | Source M2 REPORT: isolated Vulkan/CUDA/video PASS; combined load FAIL, >111 s encode stall, 3 event153, device lost/CUDA999, no new host Xid; M1 hardware unrun | EXCLUDE, owner decision pending. Failure attribution to mirror alone remains INFERRED without a matched control; isolated passes cannot certify mixed load. |
| CODE READ: trap experiments `claude/trap-experiments-20261019`, `a80249eff` | Source README: `diag-trap-exp` feature and default-off injection/spinner controls; spinner variants did not run a benchmark; inherits TSG mirror | EXCLUDE whole branch. Experimental timing/interrupt behaviour, no production need. A compile-time gate does not make its experimental base acceptable. |
| CODE READ: R1 `codex/trap-research-20261019`, `a61ade725` | Source REPORT: interrupt oracle/replay tests, compile/runtime-off `diag-intr-audit`, source mutation tests; feature branch inherits TSG work. No candidate-5 versus imported-default production binary equivalence receipt was found in the report | EXCLUDE runtime audit and full branch. Compile gating alone is insufficient for the requested byte/behaviour equivalence claim. Test-only extraction remains a later option after base separation and production elimination proof; no interrupt relay change authorized here. |
| CODE READ: K1 `codex/kf-prof-20261019`, `a1139d682` | Source README: partial core, no green committed implementation; header mutant killed by only 1 test; collector/wrappers/integration/overhead unrun | EXCLUDE now. **Next topic** after beta correctness/installation: finish the profiler under its original mutation gate, then prove production feature elimination and review independently. |
| CODE READ: H1 `codex/hitch-timeline-20261019`, `f714153ff` | Source README: offline tools 84 pass / 18 mutants; runtime recorder stopped on missing broker/clock contract; no hardware run | EXCLUDE recorder and branch. Offline analysis is useful evidence tooling, not a ready production recorder. Do not invent the missing clock contract. |
| CODE READ: L6 `codex/prewarm-20261018`, `6999c303b` | Source README: investigation only; one whole-memfd descriptor, mmap-lock causal duration unproved, registration chunking conflicts with single-handle consumers | EXCLUDE analysis-only topic from product. Preserve its constraints as review input. |
| CODE READ: separate prepopulation fix `claude/prewarm-fix-20261018`, `d8bff5109` | Five-file diff adds fallocate population in 64 MiB chunks before the existing one registration; unlike registration splitting it retains the single descriptor | DEFER. A concrete fix exists independently of L6, but no C6 hardware or independent acceptance establishes mmap-lock/tick effect; fallback still issues the whole registration. Review source tests/unsafe boundary and measure on a slot before promotion. No timer workaround. |
| CODE READ: L3 `codex/hang3004-20261018`, `e679e927d` | Two-file diff adds batch tests/controls; no production fix | EXCLUDE from product assembly; analysis does not establish a low-VA leak cure or explain long-use degradation. |
| CODE READ: P1 `codex/perf-parity-20261019`, `1fa726fee`; LX1 `codex/linux-parity-20261019`, `ace63a5ad` | Remote heads are reports of blocked restoration / failed stock-guest prerequisite, not an accepted fix | EXCLUDE as implementation topics. Their outcomes remain acceptance risks; do not rewrite a failed prerequisite as a beta pass. |
| CODE READ: old `codex/gsp-heartbeat-clock-20261016`, `ac01b7bd3` | Sweep's protected-decision and conflict records supersede old blocking heartbeat imports | EXCLUDE old standalone merge; only the approved nonblocking replacement is selected. |

## Integration order and stop boundaries

CODE READ: preserve candidate-5 first; queued order is isolated F-2, CE2, L5
seed retirement, L5 stale-life cancellation, then the approved nonblocking
heartbeat series after checking its exact fn72/ABI dependencies. Keep each topic's
EXPECTED RED history separate from its fix and use one topic per commit group.
Rerun affected full release suites, retain candidate-5 test names/counts, and
independently require at least two killing tests for every imported mutant.
Do not treat these requirements as completed by reading another lane's report.

CODE READ: [CONFLICTS.md](CONFLICTS.md) is the resolution ledger. An unresolved
change in display pacing/tick source, broker/cursor, USER relay/lazy observe,
channel birth/TSG/latejoin, doorbell/drainer, boot-layer scrub or interrupt relay
requires STOP and a concrete report. Never resolve such a conflict by guessing.

INFERRED acceptance falsifier for every queued pure fix: a changed authority
boundary, lost candidate-5 test, mutant with fewer than two killing tests,
unresolved protected conflict, or hardware regression beyond measured spread
invalidates promotion. Hardware remains unscheduled, n=0 for C6.

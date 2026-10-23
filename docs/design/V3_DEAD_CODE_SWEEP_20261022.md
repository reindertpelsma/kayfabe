# C1 documentation and dead-code sweep — 2026-10-22

**STATUS: RESEARCH, 2026-10-22.**

CODE READ: audited product snapshot `e64b8e5f6` (master at lane start), not the
unmerged candidates. No Rust source, manifest, workflow, root README or protected
PR file was changed. The evidence bundle is
[`traces/cleanup_sweep_20261018/README.md`](../../traces/cleanup_sweep_20261018/README.md).
The lane preserves archive/ and traces/; no deletion is justified by this sweep.

## Method and falsifiers

INFERRED H1, registered before inspection: missing opening metadata can be
restored mechanically. FALSIFIER: no explicit allowed state/date in that file or
its history. Outcome: promote existing statements only; leave classifications
that require judgement in the queue below. Dates describe the original statement,
not new hardware verification.

INFERRED H2, registered before inspection: some candidates have no callers.
FALSIFIER: any manifest edge, CI/script/doc reference, test discovery, external
entry point, baseline entry or evidence/history role. Outcome: the apparent
orphans below fail the deletion proof or require a protected-baseline edit.

INFERRED H3, registered before inspection: some README.md / STATUS_DETAIL.md
claims cite absent or superseded evidence. FALSIFIER: the path exists at the audited revision and the citation
still describes its explicitly historical measurement. Outcome: zero missing
literal citations found; older current-state sentences need context, not erased
history or relabelled measurements.

MEASURED (2026-10-22): `cargo check --locked --release -j2 --all-targets -p <crate>
--message-format=json` completed for all 39 workspace crates with
`RUSTFLAGS='-W dead_code'`, under the prescribed cargo flock, nice level and private
target directory. There were 25 unique dead_code diagnostics. The release Clippy
workspace/all-targets run completed; `scripts/ci/debt.py clippy` reported
`existing-debt=532 new=0 resolved=0`. No lint baseline was changed. Exact crate
completion/error records are in `cargo_dead_code.json` in the evidence bundle.

CODE READ: reference discovery covers 2,564 tracked text files up to 3 MB each,
excluding archive/, traces/, third_party/ and docs/archive/. Manifest traversal
includes normal, dev, build, target-specific and renamed edges through the
existing dependency checker. Searches produce candidates, not a cargo-udeps proof:
public APIs, runtime loading, generated names and external callers can escape
literal searches. Cargo checked default features on this host, not every feature
combination or target architecture.

## Candidate table

CODE READ: every compiler diagnostic below is already bound by path/category/text
in `scripts/ci/clippy-debt.json`. That baseline is protected by PR #71; this lane
cannot remove a site and update its baseline test-first. The full 25-diagnostic
inventory and all 85 script candidates are retained in `cargo_dead_code.json`
and `reference_sweep.json` in the evidence bundle.

| Item | Evidence concerning references | Risk | Proposed action |
|---|---|---|---|
| `kf-host/src/lib.rs::host_version_gate` | CODE READ: private function; no call in that crate; compiler flags it. Its name remains in a rustdoc link. | Baseline bound; removing it needs a doc-link correction too. | Separate tests-first cleanup once baseline protection is lifted. |
| `kf-disp/src/pace.rs` test helpers `head`, `sent` | CODE READ: two private helpers have no test calls and produce two diagnostics. | Baseline bound; surrounding pacing tests are active. | Remove only those helpers in a separately verified follow-up. |
| `kf-disp/src/engine/lockstep_soak.rs` | CODE READ: 14 diagnostics for unused event payloads, trace fields and `CALLS`; the soak itself is active. | Harness mutation sensitivity; baseline bound. | Review whether payloads should strengthen assertions before deleting them. |
| `kf-qemu/src/userbirth.rs::is_current` | CODE READ: compiler flags the unused method. | Baseline bound and protected by the #71 file set. | Leave to the channel-lifetime owner. |
| `kf-cuda/src/driver_unsafe.rs::{cuMemUnmap,cuMemAddressFree}` | CODE READ: dynamically loaded fields are initialized but not read; compiler flags them together. | Unsafe perimeter and CUDA allocation lifetime; baseline bound. | Audit resource-release intent before any removal. |
| `kayfabe-device/src/ga10x.rs::{WPR2_LO_UP,WPR2_HI_UP}` | CODE READ: default library check warns, but tests use both at lines 2079/2083. | False positive if production-only liveness is treated as whole-tree liveness. | Keep; optionally scope to tests in a baseline-aware follow-up. |
| `kayfabe-isolate-host/src/rm.rs::{release_birth_space,birth_space_census}` | CODE READ: one combined compiler diagnostic; comments still describe the release contract. | Raw-ladder dependency; baseline bound. | Check missing lifetime behavior before treating these as removable. |
| `kayfabe-isolate-host/src/rm.rs::HandedClient::minted_by` | CODE READ: compiler flags it, but `tests/own_client_invariant.rs:994` asserts its source spelling. | Removing it breaks a current invariant test; baseline bound. | Keep until the invariant test is redesigned test-first. |
| `kayfabe-rm-ladder/src/main.rs::P2_COPY_ENGINE` | CODE READ: compiler flags the constant; nearby comments retain the historical family explanation. | Required raw grader; baseline bound. | Small follow-up with raw-client checks, not a crate deletion. |
| `kayfabe-cuda/src/driver_unsafe.rs` unmap/free fields | CODE READ: same unused-field diagnostic as the v3 binding. | Optional frozen CUDA path; baseline bound. | Keep with the required dependency closure. |
| Suppressed `kf-cuda` async copy adapters | CODE READ: `memcpy_h2d_async` / `memcpy_d2h_async` explicitly allow dead_code and document retained audited/harness roles. | Warning sweep alone cannot see them; unsafe review history. | Preserve; require an audit decision before deleting perimeter documentation. |
| Suppressed `kf-host::UsermodeWindow::object` | CODE READ: field comment explains handle retention; warning suppression is explicit. | Ownership/lifetime fields need not be read to matter. | Keep. |
| Suppressed `kf-qemu` USER-store adapters | CODE READ: explicit next-caller and cleanup contracts; file protected by #71. | Resource ownership and in-flight development. | Keep; no deadness claim from the lint suppression. |
| All 17 `kayfabe-*` crates | CODE READ: zero `kf-*` → `kayfabe-*` manifest edges, but all 17 are reachable from `kayfabe-rm-ladder` including optional/dev edges. | Deleting any requires analysing the required raw ladder, not merely the v3 product. | Keep all; the hypothesized orphan crate set is empty. |
| `force-host-page-size` in both raw crates | CODE READ: cfg reads exist in `src/page_size.rs:106`; no literal launcher caller found. | Compile-time test seam, not a runtime env flag. | Keep unless all downstream/test users are ruled out. |
| `kf-chan/lifecycle-mutants` | CODE READ: `scripts/lifecycle_mutation_check.sh` explicitly enables it. | Required mutation tooling. | Keep. |
| `kf-oprom/alloc` | CODE READ: default feature and live cfgs in lib/pe/rom; firmware shares this crate. | no_std/allocation contract. | Keep. |
| `kayfabe-rm-ladder/cuda-window`, isolate `cuda-scratchpad` | CODE READ: feature edge connects the raw ladder to the optional CUDA isolate and binding. | Optional grader mode; default check is insufficient to establish deadness. | Keep. |
| `kayfabe-device/test-lock-probe` | CODE READ: no active-workspace caller, but `archive/tests/Cargo.toml:48` enables it. | Historical rank falsifier; explicitly outside deletion scope. | Keep; the no-reference hypothesis is falsified. |
| 85 scripts with no other literal basename reference | CODE READ: 575 script files scanned; 78 also have no stem references in the limited live-text universe. | Python test discovery, dynamically composed hook paths, manual/external invocation and history. | Candidate inventory only; no bulk deletion. |
| `scripts/ci/test_*.py`, appmatrix tests | CODE READ: several appear in that 85-script set, but unittest discovery supplies their callers. | Deleting them silently weakens CI. | Keep; search absence is a false positive. |
| `scripts/bench/blockage_hook.sh` and old raw hooks | CODE READ: no external basename/stem match in the live-text search; headers describe historical two-arm experiments. | Evidence-generating instruments/history. | Preserve and cross-link if an owner index is later requested. |
| `scripts/bench/gpu_fault_containment.sh` | CODE READ: no external basename/stem match in the live-text search; header provides a manual invocation and test contract. | Security experiment entry point; not executed in this GPU-free lane. | Keep. |
| `scripts/build_arch_pdf{,_v3}.sh` | CODE READ: no external basename/stem references found; scripts assemble design bundles from named sources. | Manual authoring tools; no proof that external users stopped invoking them. | Keep or document their entry points; do not delete design history. |
| 35 doc-only `KF3_*` tokens | CODE READ: not present in scanned non-doc text; inventory includes prefixes and proposed names, not just complete flags. | Misclassifying proposals as removed runtime functionality. | Preserve classification/history; never delete diagnostic flags from this search. |
| `docs/design/V3_GPU_FAIR_SHARE.md` | CODE READ: no other live-text basename reference found among the original 128 design docs; it records an owner idea with DESIGN-ONLY status. | Design/history, not dead executable code. | Preserve; optionally add a roadmap link in a future index edit. |
| `docs/design/V3_WINDOWS_POOL_AUDIT_20261005.md` | CODE READ: no other live-text basename reference found; a dated RESEARCH audit. | Audit evidence remains valuable without inbound links. | Preserve; do not delete a historical negative result. |
| Proposed `KF3_DIAG_MAPLOG`, `KF3_DIAG_PT_STALL_SNAPSHOT`, `KF3_DIAGNOSTICS` | CODE READ: the flag inventory proposes these names; no implementation read site found in the limited search. | Proposals, not dead runtime readers. | Keep the proposal status explicit. |
| `KF3_RC_RECOVERY_ENABLED_DIAG` | CODE READ: the flag inventory already says “no code” / historical command record only. | Removing the history would hide the original finding. | Keep the warning; do not advertise it as a working flag. |

INFERRED: no candidate satisfies the lane's deletion requirements while staying
within protected-file and evidence-preservation rules. No deletion commit, new
Rust behavior, timeout change or hardware experiment follows from this table.

## Documentation hygiene

MEASURED (2026-10-22): checker tests began as a separate EXPECTED RED commit
`b521f74f0` (21 of 23 failed). The resume tests-only commit `353953620` exposed two
YAML-front-matter failures among 27 tests. Checker implementation `e752e74cf`
passes all 27, and CI Python discovery passes all 66. Mutation kills are
2/3/3/18/3 (front matter/state/supersession/exit/order); the previously insufficient
ordering mutant now fails three tests. Counts and edits are in
`checker_mutations.json` and `validation.json` in the evidence bundle.

CODE READ: `python3 scripts/ci/doc_status_check.py` checks dated opening metadata
and explicit `Supersedes` links. It accepts leading YAML metadata and the existing
bold/blockquote status forms. It cannot decide status truth, semantic correction
placement, or supersession expressed without a recognized explicit link. It is
not a clean whole-document hygiene verdict. Its tests run through the existing CI
unittest discovery; the protected workflow was not changed to invoke a new gate.

MEASURED (2026-10-22): corrected-parser master inventory: 128 documents, 28
findings. Mechanical edits affect 18 documents: 17 opening-metadata promotions
and one reciprocal supersession notice in `docs/archive/THE_ARCHITECTURE_v2.md`.
The edited existing documents leave 10 findings in nine documents. The new sweep
report makes the final scope 129 documents. `doc_status_promotions.json` records
the source lines and git-history attribution; original paragraphs and corrections
remain intact. The earlier `doc_status_master.json` is provisional and superseded
for counts by `doc_status_before.json` (front matter fixed) and `doc_status_after.json`.

CODE READ: the following unresolved statuses are judgement items, not permission
to infer that everything built is LIVE. Suggestions are INFERRED for owner review.

| Document | Remaining finding | Proposed decision |
|---|---|---|
| `V3_BAR1_DOORBELL.md` | Nonstandard DESIGN+CODE; metadata follows a GB203 correction. | Separate built GB203 behavior from unverified Hopper; choose LIVE with explicit scope or RESEARCH. |
| `V3_DOORBELL_IOEVENTFD.md` | BUILT is not an allowed state. | LIVE with its existing hardware/rollout qualifications. |
| `V3_HEADLESS_GRAPHICS.md` | STEPS 1–5 PASS is not an allowed state. | ANSWERED for the recorded experiment, or LIVE for ongoing coverage. |
| `V3_P4_PORT_MAP.md` | PROPOSAL is not an allowed state; later implementation updates exist. | Choose RESEARCH/reference versus SUPERSEDED after comparing build-plan scope. |
| `V3_P5_PORT_MAP.md` | Same proposal/implementation-history issue. | Preserve the porting record; determine the authoritative successor first. |
| `V3_SECURITY_AUDIT_PLAN.md` | PLAN is not an allowed state; partial audit findings are recorded. | LIVE plan versus DESIGN-ONLY stage 2 requires scope clarification. |
| `V3_SWEEP_AND_INSTALL.md` | DESIGN is not an allowed state; implementation/history has accumulated. | Decide document scope before applying DESIGN-ONLY to the whole file. |
| `V3_USER_RING_LIFECYCLE_REVIEW1_20261016.md` | No STATUS line; dated review and later fixes do not define its current authority. | RESEARCH as preserved review; mark superseded findings individually. |
| `V3_UVM_B3_IMPLEMENTATION.md` | HOST-ONLY PROOF DONE is not an allowed state. | ANSWERED for the host-only proof, with guest integration explicitly excluded. |

CODE READ: semantic review remains open for correction order and implied
supersession across all 129 documents. This sweep makes no design change to resolve
those questions. Existing sections and original paragraphs were not reordered; new metadata does not reinstate the older architecture or invalidate later rulings.

## Stale claims and proposed owner text

MEASURED (2026-10-22): at `e64b8e5f6`, all 19 README and 59 STATUS_DETAIL literal
Markdown/inline evidence-path checks resolve. No missing literal cited path was
found. This excludes fragment-anchor correctness, shorthand/glob expansion,
external URLs and the truth of evidence contents. Exact paths/line numbers are in
`stale_claim_paths.json` in the evidence bundle.

CODE READ: the following revision/context findings do not make older hardware
measurements false. STATUS_DETAIL already opens with a 2026-10-15 warning that
its section 0 is historical; preserve that history and strengthen navigation.
Root README edits below are proposals only.

| Source at audited master | Finding | Proposed text/action |
|---|---|---|
| `README.md:10` | `89fc8a222` is a valid ancestor, not the audited master head. Product changes followed it; its measurement is not a rerun on current master. | “Master at this audit is `e64b8e5f6` (2026-10-21). The relay-fairness measurements below were made at `89fc8a222`; later changes have separate validation.” Owner should refresh the head when applying. |
| `README.md:51,56` | `eb9f2aa3` app matrix and `89fc8a222` mixed-use result are explicitly scoped historical references. | Keep their revisions; add current-head qualification rather than replacing their measurement revisions. |
| `README.md:61,63,64` | SolidWorks, 3DMark and OBS explicitly say evidence/results are not committed. No missing path is asserted. | Keep this qualification until the owner commits public-safe evidence; do not promote to reproduced results. |
| `docs/STATUS_DETAIL.md:47` | “master is the code of afb552ea” is superseded as a current-state sentence; it is already under the historical section warning. | Proposed note above §0: “Historical snapshot at `afb552ea`; current source state is indexed by STATUS_AND_HANDOFF §0.” |
| `docs/STATUS_DETAIL.md:103` | “Non-nested hardware has not been measured” belongs to the old snapshot; newer Windows/Heaven references appear in README. | Scope to the 2026-09-30 Linux measurement set. Do not infer a new Linux parity result. |
| `docs/STATUS_DETAIL.md:110,337` | “Windows research only” is superseded by later evidence named in this file's own opening update and README. | Preserve the historical sentences; point readers to the later Windows evidence. |
| `docs/STATUS_DETAIL.md:312` | Turing “never booted” is superseded by this file's §0 Turing evidence link. | Preserve historical family table; add a current family-index pointer above it. |
| `docs/STATUS_DETAIL.md:329–336` | Display “not on master / not started” predates the display evidence linked in the same file. | Preserve the old checkpoint; send current readers to V3_DISPLAY and the current index. |
| `docs/STATUS_DETAIL.md:344` | Historical CI-red wording cannot describe today's CI merely because source ancestry is known. | Keep dated history; query the actual current workflow result before asserting live CI status. |

## Proposed handoff §0 insertion for the owner

CODE READ: `docs/STATUS_AND_HANDOFF.md` is protected by PRs #68/#70/#76. Its §0
starts with a 2026-10-14 Windows checkpoint while master includes 2026-10-21
integration. Insert a dated pointer immediately under §0; retain all existing
checkpoints as history. This proposal names only the audited source state, not a
fresh guest or hardware result:

> CODE READ (2026-10-22 source index): audited master snapshot `e64b8e5f6` includes
> the relay-fairness/CI-repair integration `ae6db5fe2` and the hostile-writer stress
> test bound `840ca3843` through PR #64. README is the current user-facing status;
> its hardware/app numbers remain tied to their cited revisions. STATUS_DETAIL
> §0 is the 2026-09-30 historical snapshot, as its opening correction explains.
>
> CODE READ: the 2026-10-14 and earlier checkpoints below are retained history.
> Follow the individual source/evidence documents for their later corrections;
> do not read an old branch name, proposed flag or missing STATUS classification
> as current shipping behavior. Cleanup inventory and unresolved doc decisions:
> `docs/design/V3_DEAD_CODE_SWEEP_20261022.md`. Check the live PR list before
> selecting an integration candidate; this entry does not promote any draft PR.

## Verification limits and owner decisions

INFERRED recommendation: accept the checker and mechanical metadata fixes after
review; choose the nine pending document classifications; keep the dead-code
items until their owners can update protected baselines and tests together.
No archive/evidence deletion or bulk script removal is recommended.

CODE READ: no GPU, guest, host VM, remote machine or new dependency was used.
The lane adds Python tooling and documentation only. Cargo checks do not replace
Rust runtime suites, optional-feature builds, hardware validation or a public-API
reachability proof. The checker intentionally exits 1 while the 10 documented
judgement findings remain; it does not suppress them to manufacture a green scan.

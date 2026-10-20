# C6 Part A progress

STATUS: ACTIVE / FINAL VERIFICATION RUNNING, 2026-10-20. BETA CANDIDATE;
no supported external-user install path. Deadline 06:30 CEST.

## 1. Lane, branch and request

CODE READ: C6, `integration/beta-candidate-1-20261020`, based on candidate-5
`f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`. Request: GPU-free Stages1–4,
tests first, mutation checks and push after each stage. Coordinator approved
explicit per-device guest-os=windows|linux (default Linux), separately reviewed
derived caps, and corrections to own tooling. Those decisions are resolved.
MEASURED: latest build-tools checkpoint `480a88d66` was verified on origin;
the external report records the enclosing report commit after its push.

## 2. Result (twelve lines)

- DONE — MEASURED: Stage1 ledger published first at `3c4731eec`; [current topics](TOPICS.md).
- DONE — MEASURED: mutation wrapper corrected, five regression tests; two wrapper mutants killed by two tests each.
- DONE — MEASURED: scoped caps import, profile wiring and all selected pure fixes committed; [conflicts](CONFLICTS.md) resolved within the approved scopes.
- DONE — CODE READ: 19 P19 env selectors retired; Windows property enables18 behaviours and derived caps; default Linux settings retained.
- DONE — CODE READ: mirror/trap/profiler/recorder/full sweep excluded; no captured whole-page runtime path.
- DONE — MEASURED: launcher/doctor/install/version/audit/wrapper fixture suite42/0; [install review](INSTALL_REVIEW.md).
- DONE — MEASURED: broker make check and check-sanitize pass; pacing helper fixture corrected without production timing changes.
- DONE — MEASURED: 42 changed Rust files rustfmt2024 clean; new shell wrappers/build shellcheck, flag audit and docs command/link checks pass.
- PARTIAL — MEASURED: final22-crate regression is running; CPU-only host build and real layout smoke pass; baseline3620/0/19 reproduced exactly.
- PARTIAL — CODE READ: existing cached diagnostic gates retained; universal one-relaxed-load cost guarantee is not established.
- NOT DONE — MEASURED: Stage5 hardware and clean install walk-through, Stage6 README/PR; hardware n=0.
- DONE — MEASURED: no GPU/VM action, owner-VM stop, foreign-session message, master merge or non-lane push.

## 3. Tests, mutations and flags

MEASURED baseline: all22 crates3620/0/19, each matching candidate-5;
[per-crate reference](BASELINE_COUNTS.tsv). Changed-tree counts are pending;
focused passes are not substituted for the full run.

| MEASURED campaign | Mutants | Killing tests per mutant |
|---|---:|---|
| Scoped caps | 6 | 4/6/10/6/2/2 |
| Per-device profile | 23 | each≥2 |
| F-2 address widths | 4 | 3/2/2/3 |
| CE2 owned kernel ring | 8 | 2/2/2/10/2/2/2/2 |
| L5 seed/stale-life | 2 | 2/2 |
| Nonblocking heartbeat | 10 | 7/3/2/2/2/2/2/2/2/2 |
| Launcher/install | 7 | 3/2/2/2/2/2/2 |
| Broker pacing fixture | 1 | 2 |
| Display permission preflight | 1 | 2 |
| Generated bundle data | 1 | 2 |
| Source-policy auditor | 1 | 2 |
| Mutation wrapper | 2 | 2/2 |

MEASURED: named kill receipts are the adjacent *mutations.json files. Compiler,
harness errors and insufficient kills do not count. All injected source is
restored. The corrected real layout verifies174 file hashes and all three versions,
then removes its temporary prefix; [CPU artifact identity](HOST_BUILD.json).
MEASURED: strict AD104/native oracle rerun at c27134846 remains RED by100 words;
this is the approved derived-caps limitation, not a passing equality claim.

CODE READ: [policy](../../docs/design/V3_FLAG_POLICY_BETA_20261020.md) has156 lexical
rows:18 conditional HARDWIRE ON,1 absent/OFF mirror,42 DELETE historical/metadata
rows,95 KEEP. These are not live-env-reader counts or42 removed product paths.
All19 P19 selectors are retired; none of the18 behaviours is proven redundant
across loaded workloads by the idle-only2026-10-10 ablations. Lifecycle/lazy
observe stay at candidate-5 defaults; timer mapping was already automatic.

## 4. Decisions / recommendations

1. INFERRED: allocate Stage5 only after final CPU verification; retain the [hardware plan](HW_PLAN.md), including mixed-use aging and exact binary identity.
2. INFERRED: keep explicit guest-driver and launcher-derived resources; direct Linux QOM defaults remain unchanged. Deriving pre-fn1 driver ABI from the host is not soundly established.
3. INFERRED: before release, audit inherited diagnostic costs against the requested compile-out/one-relaxed-load bound; no such universal claim is made here.
4. CODE READ: before external support, finish packaging/signing/CI, licence/provenance and install security review, telemetry/log hygiene and the clean-directory hardware walkthrough.

## 5. Not verified

MEASURED: hardware n=0, spread N/A. No C6 boot, compositor, performance, TDR/Xid,
aging, new guest install or live doctor probe. CPU-only build and install smoke use their own host prefix. Source extraction omits an unbuilt absolute upstream EDK2 Mac include
symlink; upstream archive signature has not been verified by this lane.
CODE READ minimal direct fragments: Linux `-device kf3-gpu,id=gpu0`; Windows adds
`guest-os=windows,guest-driver=580.65.06,display=on,gop=on`, host-derived resource
and broker properties. Ordinary VM args and why each remains are documented in
[RUNNING](../../docs/RUNNING_KAYFABE.md). Minimal *working* awaits Stage5.

## 6. Cleanup at this progress checkpoint

MEASURED: own local release regression is running; CPU-only host build/layout jobs exited;
own target dirs are retained until those complete. No host GPU lock was taken.
Final cleanup and origin equality will be recorded after final verification.

# C6 Part A report

STATUS: GPU-FREE IMPLEMENTATION VERIFIED / QUALIFICATION PARTIAL, 2026-10-20.
BETA CANDIDATE; no supported external-user install path or master-merge approval.

## 1. Lane, branch, head and request

MEASURED: C6, branch `integration/beta-candidate-1-20261020`, based on candidate-5
`f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e`. Product/launcher head
`7211e357a2b652c0abbc35abf07f1022fa97d582` is verified equal on origin.
The external C6 report and final message record this enclosing report commit's
exact publication head after push, avoiding a self-referential commit hash.

CODE READ: request was GPU-free Stages1–4, tests first, mutation checks and
stage pushes; hardware only in a coordinator slot. Coordinator approved the
explicit per-device `guest-os=windows|linux` (default Linux), a separately
reviewed sweep-derived caps replacement, and own-tool corrections. Those
choices are resolved. No hardware slot was granted; deadline06:30 CEST.

## 2. Result (fourteen lines)

- DONE — MEASURED: Stage1 [topic ledger](TOPICS.md) published first at `3c4731eec`; no product import preceded it.
- DONE — CODE READ: Windows uses `guest-os=windows`,18 compatibility settings and derived caps; no P19 environment selector is read. Linux compatibility defaults stay unchanged.
- DONE — MEASURED: [flag policy](../../docs/design/V3_FLAG_POLICY_BETA_20261020.md) audits155 source/script lexical names against156 rows; no unregistered name.
- DONE — MEASURED: scoped caps, F-2, CE2, L5 and nonblocking heartbeat integrated separately; [conflicts](CONFLICTS.md) resolved within approved scopes.
- DONE — CODE READ: full sweep, None scrub fixture, mirror, trap/audit runtime, profiler and recorder excluded; captured full caps page stays test-only.
- DONE — MEASURED: wrapper regression tests5/0; all67 C6 mutants killed by≥2 named tests; no build/harness fault counted.
- DONE — MEASURED: launcher/doctor/install/version/source fixture suite42/0; doctor preserves shared-memory fallback when udmabuf is unavailable.
- DONE — MEASURED: full22-crate release run3713/0/21 versus3620/0/19; no crate count decreases; post-constructor QEMU retest398/0/0.
- DONE — MEASURED: broker check/sanitizers,42-file rustfmt, changed-shell baseline comparison, docs checks, source/secret audits pass.
- DONE — MEASURED: clippy baseline equality:423 normalized sites, zero new;21 crates exit0; raw QEMU retains the baseline89 denied diagnostics.
- DONE — MEASURED: final CPU binaries and174-file private-prefix install/uninstall smoke pass at7211e357a; upstream source signature verified.
- PARTIAL — CODE READ: universal compile-out/one-relaxed-load diagnostic cost qualification remains [unproved](DIAGNOSTIC_COST_LIMIT.md); no zero-overhead claim.
- PARTIAL — CODE READ: direct Linux QOM defaults remain; launcher derives host resource values. Guest ABI remains explicit because sound pre-fn1 host-only derivation is not established.
- NOT DONE — MEASURED: Stage5 hardware/clean new-guest walkthrough and Stage6 README/master PR; hardware n=0. No trusted-host GPU/VM action, owner-VM stop, foreign-session message or non-lane push.

## 3. Tests, mutations, flags and artifacts

MEASURED: full run at `c27134846` includes every selected behavior fix and
profile/caps change. Subsequent Rust changes are orphan-comment cleanup,
FFI safety-comment placement and grouping unchanged display constructor inputs;
the affected QEMU suite was rerun at `896cfea8c`,398/0/0. Final7211e357a adds
only launcher/docs corrections; all42 Python tests were rerun. Exact commands,
log hashes and limits: [FINAL_CHECKS.json](FINAL_CHECKS.json),
[full run](final-all.json), [QEMU retest](final-qemu.json).

| Crate (MEASURED) | Before pass | After pass | After ignored |
|---|---:|---:|---:|
| kf-abi | 592 | 601 | 0 |
| kf-arch | 4 | 4 | 0 |
| kf-broker | 162 | 162 | 9 |
| kf-census | 50 | 50 | 1 |
| kf-chan | 521 | 557 | 3 |
| kf-chip | 72 | 72 | 0 |
| kf-core | 50 | 50 | 0 |
| kf-crec | 20 | 20 | 0 |
| kf-cuda | 74 | 74 | 0 |
| kf-disp | 247 | 265 | 5 |
| kf-gop-image | 8 | 8 | 0 |
| kf-gsp | 49 | 56 | 0 |
| kf-harness | 12 | 12 | 0 |
| kf-host | 66 | 66 | 0 |
| kf-linux-raw | 174 | 174 | 0 |
| kf-mem | 186 | 188 | 3 |
| kf-oprom | 36 | 36 | 0 |
| kf-qemu | 377 | 398 | 0 |
| kf-rm | 750 | 750 | 0 |
| kf-trace | 0 | 0 | 0 |
| kf-trap | 111 | 111 | 0 |
| kf-util | 59 | 59 | 0 |
| Total | 3620 | 3713 | 21 |

MEASURED: zero failures before/after; original ignored count19, with two
additional inherited caps diagnostic emitters. All341 syntactically detected
existing test names in changed Rust files remain; probe fixture setup moves
from retired env selectors to isolated Settings, retaining assertions.
[Machine-readable counts](FINAL_COUNTS.tsv), [retention](TEST_RETENTION.json).

| C6 campaign (MEASURED) | Mutants | Named killing tests per mutant |
|---|---:|---|
| Scoped caps | 6 | 4/6/10/6/2/2 |
| Per-device profile | 23 | each≥2; one has3 |
| F-2 address widths | 4 | 3/2/2/3 |
| CE2 owned kernel ring | 8 | 2/2/2/10/2/2/2/2 |
| L5 seed/stale life | 2 | 2/2 |
| Nonblocking heartbeat | 10 | 7/3/2/2/2/2/2/2/2/2 |
| Launcher/install | 7 | 3/2/2/2/2/2/2 |
| Broker pacing fixture | 1 | 2 |
| Optional udmabuf warning / valid fallback | 2 | 2/2 |
| Mutation wrapper | 2 | 2/2 |
| Source-policy auditor | 1 | 2 |
| Generated bundle data | 1 | 2 |

MEASURED: [mutation summary](MUTATION_SUMMARY.json) links all named receipts.
All injected source is restored. Wrapper rejects compiler/harness faults and
insufficient kills. An initial broker decimation fixture failed57/60; two
failing-first wire-ack tests proved its scheduling assumption, and its fixture
was corrected without production pacing/timer changes. Both broker check modes
then pass, including778558 cursor/geometry/XOR/tick checks and29 inherited XOR
mutants. These are CPU test-backend results, not compositor measurements.

MEASURED: [clippy comparison](CLIPPY_COMPARE.json) preserves default deny rules.
Raw QEMU clippy exits101 in both trees; baseline equality is not a globally
green lint claim. Separate successful21-crate all-target checks complete the
comparison. Early stale-metadata and incomplete-export attempts are excluded.
[Shellcheck](SHELLCHECK.json) has zero new diagnostics;30 existing info sites
remain in old scripts. New wrappers/builder are clean. Docs checker verifies
6 local links,7 shell blocks and3 repository command paths, executing no VM or
install. Every staged diff passed the bounded secret/private-path scan.

MEASURED: lexical source-name counts155→155 because retired names remain in
migration metadata/history; actual P19 environment selectors retired19.
Policy156 rows:18 conditional HARDWIRE ON,1 mirror HARDWIRE OFF/absent,
42 DELETE historical/metadata rows,95 KEEP. This does not mean42 product paths
were deleted. CODE READ: idle-only2026-10-10 ablations do not prove any of the18
functional settings redundant across loaded workloads. Captured CAPS_PROBE is
replaced, not hardwired; lifecycle/lazy-observe defaults remain and timer mapping
was already automatic. [Audit](FLAG_AUDIT.json), [policy TSV](../../docs/design/V3_FLAG_POLICY_BETA_20261020.tsv).

MEASURED CPU artifact: `/data/codex-C6/kf3-bins/7211e357/`, matching QEMU and
broker version0.1.0-beta.1; firmware/bundle links point to its own QEMU build.
QEMU SHA256 `2bd1de507da1fb3e691829e70ab35d31c73506ab918207927d512e4bbe332298`;
broker SHA256 `cb2a3b3197bf4bdd6077867c004e606483b9362b114d098807712ec1e1466f0c`.
Self-contained staging: `/data/codex-C6/release-layout-7211e357a`.
174 manifest hashes, version checks, install/uninstall pass; temporary prefix
removed. [Build identity and signature provenance](HOST_BUILD.json).

## 4. Decisions the owner/coordinator must take

1. INFERRED recommendation: allocate a Stage5 slot for the exact final artifacts and [hardware plan](HW_PLAN.md), including zero-env Windows, Linux CUDA/broker, matched metrics and mixed-use aging.
2. INFERRED recommendation: complete inherited diagnostic gate cost qualification before release/promotion; retain owner-mandated names and do not claim universal zero overhead.
3. INFERRED recommendation: retain conservative explicit guest ABI and launcher-derived resources for this candidate; review early guest-fact derivation before changing direct Linux QOM defaults.
4. CODE READ: external support still needs packaging/relocation and signing, CI, licence/provenance and install security reviews, telemetry/log hygiene and unfamiliar-person clean-install acceptance. Stage6 needs hardware pass plus coordinator go.

CODE READ decisions already taken, not reopened: explicit OS property/default
Linux; scoped derived caps rather than a captured page; preserve18 functional
settings behind Windows; exclude unvalidated topics; correct own wrappers,
fixture pacing, generated bundle data and optional-tier preflight without
changing production timers, relay, cursor or scheduling policy.

## 5. What could not be verified / minimal commands

MEASURED: hardware n=0, spread N/A. No trusted-host C6 guest boot, real compositor, CUDA workload,
Heaven/Minecraft performance, TDR/Xid, aging, live doctor probe or new-guest
walkthrough. Strict AD104/native oracle remains RED by100 words, as expected;
passing derivation tests do not imply native equality. This limitation was
explicitly retained with the coordinator-approved derived replacement.

CODE READ minimal direct Linux fragment: `-device kf3-gpu,id=gpu0`.
Windows fragment: `-device kf3-gpu,id=gpu0,guest-os=windows,guest-driver=580.65.06,display=on,gop=on`
plus launcher-derived resource and broker socket/uid values and ordinary VM args.
Operator interfaces, without KF3 env settings:

```sh
bash scripts/kayfabe-run.sh --disk /path/to/linux.qcow2 --guest-driver 580.159.04 --no-display --print
bash scripts/kayfabe-run.sh --os windows --disk /path/to/windows.qcow2 --guest-driver 580.65.06 --print
```

CODE READ: these `--print` commands perform read-only host discovery unless
using offline facts; C6 tests used fixtures, not the occupied GPU. Minimal
*working* and literal unfamiliar-person install acceptance await Stage5.
[INSTALL](../../docs/INSTALL.md), [RUNNING](../../docs/RUNNING_KAYFABE.md) explain
the broker, headless and diagnostic VNC paths, explicit guest ABI, resource
budgets, UEFI/TPM, retained --no-reboot/relaunch behavior and historical rootless
facts. Support matrix rows are STALE/UNTESTED for C6; no additional combination
is certified. Source archive signature verifies against the release fingerprint
published by [QEMU](https://www.qemu.org/download/); the unbuilt absolute upstream
EDK2 Mac include symlink was omitted during safe extraction. Staged binaries
remain unsigned; no portable glibc/library floor or universal cost bound is claimed.

## 6. Cleanup

MEASURED: no C6 build/test/broker/TPM/GPG job remains. Dev and host Cargo target
dirs, including temporary baseline/lint subtrees, are deleted; local broker
outputs cleaned. Local cargo/own QEMU build locks released; GPU lock never
acquired. Versioned binaries, final staging, own QEMU source/build firmware tree
and small text evidence remain. QEMU build must remain for the requested
pc-bios/qemu-bundle links. No foreign PID, worktree, branch or target was changed.

**STATUS: RESEARCH, 2026-10-22 CEST (measurements 2026-10-21 UTC).** SW2 continuation of
`V3_BLACKWELL_RESUME_20261021.md`, transferred lease57117614 only. Original Claude branch is read-only;
all new commits are on `codex/bw-resume-cont-20261022`. Evidence prefix:
`traces/blackwell_resume_cont_20261022/`. No hardware acceptance PASS is established.

**2026-10-22 correction / current continuation:** latest owner scope is documentation and derivation
only; no hardware or rentals. Clean CAPC draft [PR #69](https://github.com/reindertpelsma/kayfabe/pull/69)
retains test-first history and explicitly distinguishes the public layout from the inferred precision-14
rule; FVLUT and complete numeric fidelity remain unresolved. See
[V3_PRECOMP_CSC_PRECISION_20261022.md](V3_PRECOMP_CSC_PRECISION_20261022.md).
The native fault's strongest CODE READ hypothesis is the native CE fixture's old C7B5 bind on a
Blackwell CE ring; causal sufficiency and the physical-channel mapping remain UNVERIFIED. No fault
is excused. The classifier/controller is not ready for reuse: parsing, ownership, fault-time init
proof and full native-to-arm coverage require completion. See
[V3_BLACKWELL_GUARD_REVIEW_20261022.md](V3_BLACKWELL_GUARD_REVIEW_20261022.md) and the
[future validation plan](V3_BLACKWELL_NEXT_VALIDATION_20261022.md).
The owner raised the lane hard stop to **7 USD total**, keeping the 5 USD credit reserve; this
supersedes the old cap4/hardstop6 language below. Spend remains 2.640452 USD; no new lease.
The earlier “Cause is UNVERIFIED” remains true for hardware causality; the new source diagnosis
narrows the next experiment. The old “remaining decisions” paragraph below is historical.


The bisect isolated PRECOMP **CAPC** as a whole: replacing only those eight words moved ON beyond the
fourth cursor allocation and into the same TDR/unload path as OFF. The derived-only control and CAPA+CAPB
comparison still bugchecked0x50. A subsequent full-boot kernel audit found an unmatched native Xid32
**before** these arms, so the comparisons remain exploratory on a contaminated baseline. No second lease
or new hardware arm was started after takeover. The transferred lease is destroyed and API-absent.

## Measurements and limits

All five new arms used diagnostic runtime `2c30a158c170a66baa37b77c19000993af1546a6`, QEMU SHA256
`dfde463c9881e44525de4eaf91902ad5346e5653bc6236d7f279feaed4479864`, the inherited printed19 flags plus
RPC/body metadata capture and progress1000. The diagnostic caps-file hook stays on its separate branch;
it is not imported into product code here. Per-process flags/QOM and binary receipts are in each arm.
The OFF process includes the approved empty GOP ROM override; its process flags record completion probe500.

| CODE READ (commit 007ba1eb0:37): quoted historical statement, evidence not revalidated by BW9.  Arm | Variable relative to control | MEASURED result | Canonical RPC count | Scope |
|---|---|---|---|---|
| earlier `on_a` | PR65 width, no page replacement | 0x50 after fourthCA7A |278| Earlier lease57108853; original evidence retained in parent branch |
| earlier `on_b` | Full PRECOMP region from diagnostic die read | No0x50, then0x116/Xid pair |4580| Earlier lease; original narrow exporter counted4577, timing parser includes Other(4) |
| `on_c0` | Derived page through same hook |0x50, last ordinal277/CA7A|278| n=1 |
| `on_c1` | CAPA+CAPB+CAPC |0x116 after init progressed, Xid32 pair|4542| No CIM Code0 observation; lane's FIXED label means only no0x50 and>=1000RPC |
| `on_c2` | CAPA+CAPB |0x50, last ordinal277/CA7A|278| n=1 |
| `on_c3` | CAPC only |0x116 after init progressed, Xid32 pair|4519| Same limited init-progress conclusion, not validation PASS |
| `off_c` | Display/GOPoff, completion probe500 | Actual580.88 Code0, then0x116/Xid32 pair|4007| No600s hold/apps PASS |

The controller used three bisect comparisons after one control, then OFF; the <=5 bisect bound held.
Its initial pre-provision cd warning is retained in lane.log; each later arm independently checked the
frozen checkout and binary, so the warning is not hidden or treated as a completed arm.
`page_change_receipt.json` proves on_c3 changed only offsets0x788+0x20*i for i0..7 against control.
Only file hashes and changed offsets are exported, not the diagnostic native page. Native raw30/30,
CUDA4/4 and gates9/9+10 individually passed on this lease. This differs from the earlier lease's29/30
native suite and does not erase its previously reported flake.

Every current canonical timing receipt reports zero unparsed RPC lines. Preserve the narrower old
api.tsv/summary.json observations: OFF4004 versus canonical4007 is three omitted Other(4) records;
canonical reset ordinals3673..3676, teardown3686 and unload4006 are used below. Preserve the old
summary's3683-teardown/4003-unload ordinals as legacy observations, not canonical references.
`arm_verdicts.json` supplies canonical data, QMP panic times/parameters and exact Xid records.

## Native fault and teardown

**MEASURED fatal baseline event:** at22:20:19.302750UTC, native `kf-gr-tier` PID90663, Xid32,
channel0x0d00000a, `intr1 00000010 HCE_DBG0 00000000 HCE_DBG1 0000c7b5`. It is neither the approved
QEMU intr00200000 pair nor a source-proved intentional native control. The native fixture printed
GR_TIER_VERDICT=PASS, then the inherited controller proceeded to Windows without a full-boot Xid gate.
Its negative cases are intended software refusals; no source FAULT_MARK for this native32 is asserted.
Cause is **UNVERIFIED**. Do not infer a method or ring from the numerical debug word.

This event was discovered retrospectively in the final full-boot journal export after the lane had
already finished. The per-arm followers did not cover it. Consequently their independently re-audited
CLEAN/EXPECTED results do not establish a healthy baseline. No re-run may treat it as an exception.
All subsequent hardware work stopped; future hardware requires resolving this native event and a guard
covering the gap before the first Windows arm.

CODE READ (commit 007ba1eb0:76): quoted historical statement, evidence not revalidated by BW9. Lease57117614 created22:04:32.564106UTC, auto-destroyPID2537232 armed first for170minutes, verified alive
at takeover; expiry00:54:32UTC was earlier than the handoff's approximate00:56. Controller ended23:11:18;
no QEMU remained at23:27:52. Five arm exports were pushed as4aa090bf4 before teardown. An extra final
kernel projection was exported locally before teardown but its first single-JSON-file commit invocation
was misread by the pre-existing helper as a path list; it was pushed afterward in3abf65976 with this
error disclosed, without changing the retained event. API absence23:28:51.639175UTC, timer cancelled
only afterward; all four transferred ledger IDs again absent23:33:43. No new lease.

Observed rate0.776667USD/h, conservative helper runtime estimate1.091743USD; combined lane estimate
**2.640452USD** including prior1.548709USD. Cap4/hardstop6/reserve5 unchanged. Shared credit43.890721USD
at closure is not settled or exclusively attributable spend. Cost ledger and per-ID receipt updated.
Trusted host/GPU/VM/locks untouched; no dump/executable/native page copied into git or inspected as code.

The missing SSH pin was repaired from new authenticated evidence, not bypassed: Vast's instance console
published ED25519 fingerprint SHA256:MYKNBIBfyB1we/l7f6RT3y3383miVrRVAU4k39XaG1s, exactly matching the
endpoint key. Own known_hosts/config kept StrictHostKeyChecking=yes, no agent forwarding. The earlier
pin question is resolved by that exact authoritative match; old lease keys were not reused.

## What the OFF probe shows

MEASURED: Xid32 pair23:07:37.411873/.425634UTC; first refused RESET_CHANNEL at23:07:41.829771,
canonical ordinal3673; unload4006 at23:07:45.052210; QMP0x116 at23:07:48.144427 (see exact JSON receipt).
The recovered dump header says0x116 at guest23:07:47.734598; recovery-clock offset is7.05s and is not
assumed identical to the crashing boot. The additional live dump is0x193, not a first141 timestamp.

CODE READ (commit 007ba1eb0:101): quoted historical statement, evidence not revalidated by BW9. The probe emitted eight GUEST-SILENT-AFTER-COMPLETION summaries, **zero HOST-FENCE-OVERDUE** summaries
and zero in-flight fence lines. First probe bounds are23:07:13.702358..13.702378UTC. Source probe_tick
samples currently readable Translated slots, skips busy slots and prints once per condition. It does
not identify the Xid channel or provide a continuous method tail. Therefore the expected first-overdue-GR
observation was not obtained; B1's specific diagnostic prediction is unsupported on this run. Xid before
reset/unload remains measured, but neither causal method nor affected ring is established. No completion
is forged, and no zero-probe count is used to exonerate an unsampled path.

## Source-derived candidate, with the missing functionality kept explicit

GPU-free EXPECTED RED7b8cdc550 precedes fixe80903277. Public NVIDIA ogkm580.159.04
@b81d58ee0224d1d290bef1c080592b619e184042 (MIT), [nvkms-evo3.c:374-432](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/b81d58ee0224d1d290bef1c080592b619e184042/src/nvidia-modeset/src/nvkms-evo3.c#L374) documents the existing CSC
matrices as S5.14 in S5.16 method encoding. Generated class headers name CSC00_PRECISION and
CSC01_PRECISION separately from LUT/clamp feature bits. Four T1 family-layer rows now declare14 for
those already implemented stages in main/overlay windows. Addresses and masks come from every family's
generated layout, with inactive windows zero. No per-die capture, neighboring row or new T3 data.

This is a **partial candidate**, not a sufficient0x50 fix. The diagnostic CAPC replacement also enables
FVLUT. Public CA7E defines CURVE selection, while current color::pipeline supports the legacy initialized
inline-table path and rejects those extra control bits. The exact mapping of the FVLUT capability to
these behaviors is UNVERIFIED, so the candidate keeps FVLUT/DIRECT/clamp flags absent. A captured word
cannot establish implemented engine functionality. Further work needs public-source FVLUT semantics,
the corresponding implementation where needed, or a discriminating bit-level experiment;
no extra rental is allowed past the unmatched-native-Xid STOP. No reverse engineering was needed.

CODE READ (commit 007ba1eb0:126): quoted historical statement, evidence not revalidated by BW9. MEASURED GPU-free verification: kf-disp293/0/5 ->296/0/5, three mutants each2killers, restored source,
clippy77->77/added0, rustfmt. Family listing regenerated193->225 rows with only32 new CAPC words.
The historical AD104 f5 case retains its full comparison of all other words; explicit precision checks
precede projection of only the two intentional new fields. Case renamed, none dropped/newly ignored.
This also makes AD104 hardware behavior after the declaration explicitly UNRUN. The separate bench
classifier fix cc593e3be had EXPECTED RED3547e5ca7:14->18 tests, two mutants each2killers. It now rejects
unparsed Xid records and high channels outside0x0e0000xx; it was not deployed to an already finished lane.

Remaining decisions: keep the precision candidate isolated until tested (recommended); investigate the
unmatched native32 and close pre-arm guard coverage before another lease; derive CA FVLUT semantics and implement
the corresponding behavior from public sources before claiming that feature. No approval or budget increase is assumed.

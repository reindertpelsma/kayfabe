# Bounded current-source family sweep

STATUS: ANSWERED, 2026-10-29. Earlier bounded stage and later Stage C are closed;
no new run authority.

Update2026-10-29: the later [Stage C guard/profile proof](../sw5_stage_c_20261028/README.md)
addresses the conditional lost-create-receipt gap without changing the historical evidence
below. Its one admitted Blackwell attempt stopped on a real SSH identity refusal before
native or guest work; actual die is UNBOUND and all GPU tiers remain UNRUN/UNATTRIBUTED.
The older GA106 pass remains a separate exact-source result. No retries or identity bypass.

MEASURED (2026-10-28, [GA106 result](lease2/result.json)): GA106 passed native gates9/9,
gate10 and bare30/30, then stock Linux fast30/30 at the original180s arm limit. The source
was approved master `7dcedefea093de078188efd5f712561bde746362`, clean at build, with
host/guest driver595.91.07 open/host-mode, kernel6.8.0-59-generic and framebuffer8192MiB.
BAR1 was256MiB, unchanged. Hardware n=1; no spread, performance or fat-guest claim.

| Target | Native control | Stock Linux fast | Evidence |
| --- | --- | --- | --- |
| GA106 | 9/9+10, bare30/30 | PASS30/30 | [result](lease2/result.json) |
| GB205, unbound | Initial image failed; box excluded | UNRUN | [result](lease1/result.json) |
| GB206 candidate | No contract receipt or allocation observed | UNRUN | [STOP](lease3/STOP.json) |
| Turing | No candidate in final filtered market | UNAVAILABLE | [market](market_closeout.json) |

MEASURED (2026-10-28, [birth census](lease2/births.log), [Xid census](lease2/final_xids.json)):
161 suite and12 gate births were all USER. Kernel Xids were0 initially,5 after native controls,
and10 finally, exactly the source-approved negative controls. [Bare](lease2/bare.log) and
[fast](lease2/fast.log) logs preserve all30 per-arm verdicts; the bare log redacts its GPU UUID
with a [digest record](lease2/bare.log.privacy.json). Binary digests are in
[binary identity](lease2/binary_sha256.txt).

CODE READ (2026-10-28, [source inventory](source_inventory.json)): stock master has lifecycle
and lazy-observe controls but no TSG mirror. Full-feature correctness, Windows, ladder/apps
and display remain UNRUN. Linux relay flags do not prove Windows behavior. No experimental
source was merged. The historical family audit is not promoted to a current result.

MEASURED (2026-10-28, [GB205 STOP](lease1/STOP.json), [result](lease1/result.json)): the first
VM reached settled SSH, but its initial proprietary575.51.03 module rejected the GPU and
required open modules. Initial nvidia-smi returned no devices, with zero recorded Xids.
This stopped before planned-driver provisioning or native RM binding. Actual die is unbound;
no native-suite failure or kayfabe defect is claimed. The original STOP digest is preserved.
The machine/host remains blacklisted; this stage did not retry that box.

The [GB206 preregistration](GB206_PREREG.md) records the tested ordering correction: retain
initial image-inventory errors, install and strictly verify the planned open driver, bind
actualGB206, then require all native controls before guest work. MEASURED (2026-10-28,
[third STOP](lease3/STOP.json), [reconciliation](lease3/allocation_reconciliation.jsonl)):
the create call returned no parseable contract receipt. Repeated API queries observed no
third allocation. There was no ID, workload, retry or third destroy call. The empty-ledger
watchdog was identity-checked and retired. Its label/response receipt was not retained;
that historical receipt gap is not reconstructed. The later Stage C guards address the
conditional adapter defect; no retrospective allocation or GB206 hardware result is claimed.

MEASURED (2026-10-28, [closeout](closeout.json)): both created IDs58115039 and58117397 were
captured and SHA-verified before exact-id destruction, then verified API-absent. All3 owned
watchdogs are gone. Known retained cost is1.563996372USD, including transfer allowances and
margin, not a settled invoice. The failed-create reservation1.23078125USD is listed separately;
conservative total2.794777622USD stays below4. Shared cap12 and account reserve8 were retained.
Account-wide credit movements are not attributed to this lane.

MEASURED (2026-10-28, local verification): [guard tests](guard_tests_14.log)14/14,
[mutants](mutations.json)8/8 killed, [initial adapter](operator_tests.log)70/70 and
[third adapter/profile](operator3_tests_final.log)74/74. The2 inventory mutants each have2
killer tests. [Canonical Xid tests](xid_tests.log)109/109 and canonical payload tests208/208
passed. Tests-first commits are `927215351` and `d952b2f18`; implementations are `0372b6841`
and `9cc291a0a`. No product Rust/C code changed.

Original raw captures and preregistrations remain private. Public source inventories for
[attempt1](sources_lease1_inventory.json), [attempt2](sources_lease2_inventory.json) and
[attempt3](sources_lease3_inventory.json) distinguish executed raw digests from published
text digests after local-path redaction. Archived operator text records the run; it is not
a maintained rental service. See [privacy edits](privacy_redactions.json) and the initial
[preregistration](PREREG.md). No secrets, binaries or VM images are included.

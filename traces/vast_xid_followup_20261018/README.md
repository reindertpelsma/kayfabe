# Offline Xid/result follow-up; no rental

STATUS: REHEARSAL, 2026-10-23. Run `v9-no-rental-followup-20261023`.
No Vast API call, rental, deployment or production renting enablement in this follow-up.
[Pre-recorded hypothesis/falsifier](experiment_plan.json).

MEASURED on 2026-10-23, prior run `v9-xid-kvm-20261023`, source2c908a0e8:
the published latest lease completed raw30/30, thin30/30, census and host/fat CUDA4/4
with bound PASS. The earlier V8 lease stopped at raw arm7/defer-liveness rc6; its exact
journal was absent. The [V9 receipt](../vast_xid_v9_20261018/live_run.json) supersedes that
finding for the later checkout. This follow-up does not claim another live run.
The box README's stale latest-evidence link was corrected in place.

CODE READ: S4 exact-source admission is already integrated by fix2c908a0e8, following
standalone EXPECTED RED commits including ed0aeee72. Import56dfc55be copied the S4 module
and four original tests from12f86ae38; its subsequent per-arm API factors the original matcher.
This follow-up leaves the canonical module and payload watch_delta/xid_ok/guard_command/
guest_command/stage_grade byte-identical. [Admission provenance](admission_provenance.json).
Every other arm retains strict zero-new-Xid admission; no timer/constant or crate code changed.

MEASURED on 2026-10-23, run `v9-no-rental-replay-20261023`:136 existing guard/diagnostic
contracts pass. The retained V9 receipt hashes match; two native controls/combined five faults
admit through the original shared matcher. Wrong channel/VA/type, non-CE client and an extra
record each refuse. [Replay](guard_replay.json). No synthetic record is presented as a live fault.

MEASURED on 2026-10-23, run `v9-no-rental-followup-20261023`: EXPECTED RED76fb3b8c4
shows that log text reached the callback stream but was missing from the local structured
result; the reporter rejected an added field. The separate fix retains optional diagnostics
in `/workspace/kf-result.json`, using the existing arm decision/state and log stream.
Explicit fixture migration: one older test's hand-built count-only event now uses the real
record constructor; its original failing-arm count assertions remain intact.

CODE READ: each local record has arm/mode/rc/admitted/text/sha256/source_bytes/dropped_bytes.
It retains a256-byte clean or2048-byte refused UTF-8 head/tail preview; the hash and source size
identify the original UTF-8 input before preview truncation. Limits:128 records,49152 serialized
JSON bytes including escapes; the original65536-byte result-file cap remains. Compaction drops
older previews first and preserves the first refusal. Metadata/hashes grant no trust or PASS.
Unknown fields, malformed types/hashes and over-budget records fail closed.

CODE READ: scope is the repo-side structured result file. The reporter validates this optional
local field and retains exactly the accepted Worker wire result keys; it does not forward a
new diagnostics field. Larger per-arm context continues through existing authenticated log
frames, with the accepted Worker/runner sanitization and bounds. No Worker bundle/schema,
auth, request limit, spend policy or verdict rule changed. Remote wire-level diagnostic fields
are outside this change; the accepted Worker does not support them.

MEASURED on 2026-10-23, run `v9-no-rental-result-rehearsal-20261023`: a positive retained-receipt
replay produces PASS with two local records; a synthetic wrong-channel replay produces FAIL,
its diagnostic preview and named arm_defer_liveness count row. Local files are1118/2508 bytes.
The reporter accepts both and emits the exact existing six wire fields; text cannot produce
PASS. [Rehearsal](result_rehearsal.json), [positive local result](rehearsal_result_PASS.json),
[synthetic-refusal local result](rehearsal_result_FAIL.json). Hardware fields are copied from
V9 only to exercise schema handling; this is not a new GPU observation.

MEASURED on 2026-10-23, run `v9-no-rental-target-20261023`:480 target-OS tests pass in70.134s
(before461, after480). All61 payload/log mutations have at least two named killers each,
including11 new result-file/parser/budget/hash/truncation/wire-isolation mutants.
The remaining env8/diagnostic10/policy6/cache43 mutations also pass (128 total), each with
at least two named killers; canonical60/60 passes under its original rule.
[Preflight](preflight.json), [payload/log cases](target-payload-mutants.json), [canonical](canonical.txt).

CODE READ: no new lease is authorised. V9's live proof predates this local result-file change.
Remaining gaps stay hosted OIDC, billed Cloudflare CPU, independent SSH artifact/hardware proof,
unabridged artifacts, separate APT time/mirror and slower bench-tree setup attribution.
Deferred Worker format2/README/review lows remain; earlier MANUAL proof was not repeated.

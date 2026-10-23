# Shared exact-source Xid admission and callback diagnostics

STATUS: LIVE PASS, 2026-10-23. The single authorised V9 lease is complete; see [live evidence](LIVE_RUN.md).

CODE READ: the sole production implementation is `scripts/bench/native_xid_guard.py`,
imported verbatim with four original test files from S4 branch codex/sweep-dies-20261022
at12f86ae38a367b02f80d87e73d7daefa45efbe87 in commit56dfc55be.
[Import hashes](s4_import.json), [API-refactor provenance](shared_guard_provenance.json).
The subsequent shared per-arm API factors the original native source extraction and delegates
to its unchanged Counter matcher. Existing guest approval/supervision/source identity code is
byte-identical. No second admission matcher or per-die client table exists in the payload.

CODE READ: copy-engine controls require exact source channel, VA, fault/access type and a
HUBCLIENT_CE digit suffix, without requiring a particular CE instance. The original S4 native
missing-page GR0_PBDMA0/HUBCLIENT_ESC control remains separately exact. Authentic native proof
is four CE faults plus that one GR control. A changed/extra/missing record refuses; neither
history nor per-run source markers can be borrowed by an unrelated arm.

CODE READ: payload xid_ok delegates per-arm native decisions to the shared module. Both actual
controls then freeze the complete journal through approve_native. Thin guests use the original
S4 supervisor: live owned QEMU PID/starttime, revision/source pins, guest channel/host twin,
VA/type/client, recovery/notifier proof and immutable native history. Every other native/QEMU
arm retains zero-new-Xid requirements. The outer watcher compares the approved multiset after
stripping only dmesg/journal timestamp prefixes. Original timeout arguments are preserved.

CODE READ: per-arm watcher/serial/QEMU evidence is copied into the reporter's existing output
file with a32KiB head/tail bound, a1MiB run budget and64KiB reserved for the first refused arm.
The strict result schema stays unchanged: the failing arm appears as a named count row;
diagnostic text uses normal authenticated log chunks and is scrubbed by Worker and runner.
The local acceptance controller retains the full validated/scrubbed log, beyond the runner UI's
200-line display budget. No SSH access is required to retain the new failure evidence.

CODE READ: prior V8_CLOUDINIT preserved the arm summary only, not its exact Xid journal.
It cannot be retrospectively graded by the new matcher. Authentic S4 AD102/GA102 journals and
source-control text are replayed unchanged in substance; negative fixtures mutate one field.
Only trailing ASCII whitespace in imported control text was normalized; no Xid record changed.

CODE READ: standalone EXPECTED RED commits cover dispatch, bounded diagnostics, authentic
native controls, guest supervisor delegation/log descriptor, frozen history and a late-fault
log reserve. Explicit fixture migrations: two former marker-only positives now require refusal
without channel/recovery proof; V7 bounded_text mock forwards its limit argument; one newly
misplaced guest test method was restored to its original fixture class with assertions intact.
Four old mutation anchors now target the shared matcher rather than removed inline rules.
The mutation driver selects the guard-policy classes; all100 original S4 tests, including
source-pin cases, remain in the normal CI suite. The container Git mount uses the repository's
actual common directory; missing metadata caused five setup errors before that harness fix.

CODE READ: no Worker/reporter schema, bundle/default, product crate, source revision selection,
box secret boundary or deadline changed. [Pre-recorded hypothesis/falsifier](experiment_plan.json).
CODE READ: the 2026-10-23 experiment plan specifies cap4/stop3 USD, fixed-die/verified policy, independent reaper
before create and exact-ID teardown. The one lease is now complete; no additional lease or retry is authorised automatically.

MEASURED on 2026-10-23, run `v9-gpu-free-20261023`:461 target-OS tests pass in67.317s,
including all100 imported S4 tests. The50 payload/shared-guard,43 cache/helper and6 policy
mutants each have at least two named killers; the8 provider-environment and10 diagnostic
mutants also pass with at least two named killers each (117 total). Canonical60/60 has no problems. [Preflight](preflight.json),
[guard mutations](payload_mutants.json), [canonical ledger](canonical.txt).
MEASURED on 2026-10-23, run `v9-cloud-init-replay-20261023`: fresh network-disabled Ubuntu22
fixtures pass complete orchestration in74.132s and post-swap failure in52.815s, with current
failure hardware. GPU/kernel/VM/compiler/deep-suite leaf operations are explicit fixtures;
these are environment/orchestration checks, not GPU evidence. [Replay](cloud_init_replay.json).

MEASURED on 2026-10-23, run `v9-xid-kvm-20261023`: the bound verdict is PASS; host/fat CUDA4/4 each,
kf tests3749/0, gates9/9+10, raw30/30, thin30/30 and census pass on source2c908a0e8.
The exact instance is absent; inferred cost upper0.515590547 USD. All three hosted CI runs
passed on the same source. [Live result, bounds and every remaining gap](LIVE_RUN.md),
[structured receipt](live_run.json), [hosted checks](hosted.json).

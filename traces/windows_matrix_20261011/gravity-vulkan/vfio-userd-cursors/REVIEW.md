# Independent review of restricted user-ring models and native sampler

**STATUS: ANSWERED, 2026-10-12.** Reviewed exact096d1fac6, then controller fixes.
Reviewer: independent Codex review_late_fifo_probe session, read-only/no hardware.

No reproducible internal defect found in Ledger, strict GP descriptor decoder,
transactional stream parser or Windows construction witness.16 ring/parser model
cases and3 original sampler checks pass. This does not validate an unwritten
native adapter. Needed before wiring: VA/object-generation pins, birth-scoped
admission, GPU-read-only reserved maps/no writable aliases, unmapped private USERD
integrity, supported-class ordered tail/reference initialization, software-action
ordering and SYNC_WAIT semantics.

Sampler findings: request-only allocation attribution; missing parent-free
invalidation; oversized partial lines retrying; correlation across missing records;
summary joining by page offset instead of lifetime. Controller corrections add
unique successful empty-reply pairing, device/client/exact-free invalidation,
request indices/serials, strict line caps, candidate-identity joins and six passing
GPU-free checks. Clear/resume after gaps was rejected by a reproduced stale-reply
counterexample. Final policy stops on any discontinuity, except a queue's very
first sequence0 prefix/no missing records in this fresh cold-boot profile.
Reviewer agreed this restricted policy resolves that blocker; no private/native
privileged probe needed. The collector is not a general passive-history decoder.

Initial100000-record failure/no footer stays incomplete. Corrected collector
exports ordinary progress at100Hz, preserves initial/identity/zero transitions,
and produces a truthful limited footer at the planned cap. Hardware coverage and
all runtime integration gates remain separate; no Kayfabe Gravity pass claimed.

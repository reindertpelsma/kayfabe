**STATUS: RESEARCH, 2026-10-23 — Linux control implemented; hardware n=0.**

CODE READ, 2026-10-23: the current optional arm and trusted-host runbook are in
[`V3_TRANSLATED_RC_CONTROL_20261022.md`](../../docs/design/V3_TRANSLATED_RC_CONTROL_20261022.md).
The reset option analysis below is historical; the owner accepted continued
RESET_CHANNEL refusal. This does not block the negative-control implementation.
The code is on `codex/translated-rc-reset-20261022`, draft PR #91, above PR #71.

MEASURED 2026-10-23: 59 Python tests pass after restoring all seven
capture-guard/guest-observer mutants. Every mutant has at least two independent
killers; exact test names are in `control-python-mutations.json`. These use
synthetic captures and public-source patch fixtures, not a GPU or a guest IRQ.

MEASURED 2026-10-23: the earlier implementation checkpoint had 21/21 Rust arm
and owner-routing tests green, and feature-enabled Clippy over kf-chan, kf-qemu,
kf-harness completed with 278 warning identities versus 278 before, added=0,
errors=0. The final native-bystander edits require a fresh cargo run; these earlier
counts do not certify those edits. The probe and exact guest handshake bridge
compiled as kernel C objects against dev headers 7.0.0-38; the static owning
loader compiled with Wall/Wextra/Werror. Full patched 580.159.04 driver linking
against the chosen guest kernel remains unverified.

CODE READ test-first commit sequence, 2026-10-23:
`2ec075877` (fixed stream, one shot, scope, ordering), `bedd9a383` (GP_GET and CE),
`507bd34d6` (supported CE2 capture), `9dde1685b` (owner EIO and guest health),
`0b37d8d8a` (native-only signature characterization). Each was committed as
EXPECTED RED before its corresponding implementation. Initial Rust stub:
2 passed/15 failed; initial guard: 3 positive-capture errors; initial guest patch:
5 failures/1 error. The native profile's two positive cases were red before its fix.

MEASURED hardware acceptance as of 2026-10-23: n=0; no native Xid signature,
Linux guest receipt/isolation/teardown result, or Blackwell result is claimed.
The native/OFF/ON runbook retains existing fastguest gates and additionally
requires owner EIO, actual notifier/event, failed-work GP_GET=0, bystander GPU
progress, post-fault guest health, exact host Xid identity and no extra Xid.

CODE READ historical checkpoint follows (2026-10-22):

**STATUS: RESEARCH, 2026-10-22 — public reset contract checkpoint; no runtime change.**

CODE READ: branch starts at commit `1c6452a276d7014bece3adf42d68cbd4edffbc18`
(PR #71). Source findings, the unresolved lifecycle decision, and the blocked
hardware-arm outline are in `docs/design/V3_TRANSLATED_RC_RESET_20261022.md`.

MEASURED 2026-10-22 baseline: `cargo test --release -j2 -p kf-rm -p kf-qemu
-p kf-harness` finished successfully: 1263 passed, 0 failed, 0 ignored, 100 targets.
The invocation used the lane target, nice 19 and the required shared cargo flock.
Clippy over the same three crates with `--all-targets` finished successfully:
271 warning sites, 0 errors. Hardware n=0. No runtime edits or mutants were made.

MEASURED 2026-10-22 claims baseline: 3924 diagnostic identities; 167 are already
outside the repository's recorded debt at the branch start. This checkpoint does
not change that debt file or assert that the repository-wide claims gate is green.
New identities are compared with the saved branch-start identities, separately
from that inherited gate result. See `baseline.json` for commands and counts.

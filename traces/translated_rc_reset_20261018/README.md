**STATUS: RESEARCH, 2026-10-23 — final GPU-free bar green; Linux GPU acceptance pending.**

MEASURED final source `9ca3aaada7cdfa98b230c6881612d2d6e040812b`, 2026-10-23:
22 kf-* crates, **4020 passed, 0 failed, 22 ignored, 236 targets**. Clippy:
278→277 diagnostic identities, added=0/errors=0. Claims: 3924→3924, added=0;
inherited recorded-debt discrepancy unchanged. Python: **63 passed/0 failed**.
All **15 mutants** (six Rust, nine Python) have at least two killers. Exact
commands, input hashes, counts and scope are in `control-final.json`; build-farm
claims separately cite BF_HEAD `1c78c481e783b6f2b44a86ed5027a14e5ed7c779`.
This result supersedes the in-progress and stopped checkpoints below.

MEASURED cleanup, 2026-10-23: the dev target and guest C build tree were deleted
using find-delete semantics. No owned workload or resource lock remains. The
controller's build-farm lane cache is retained: cleanup returned BF_REFUSED/75
during Q4's quiet window after bounded retries. The quiet flag was never changed.
Hardware GPU-control runs remain n=0; full driver/probe linking against the chosen
guest kernel and native/OFF/ON boots are the remaining acceptance procedure.

**STATUS: RESEARCH, 2026-10-23 — final guest-control qualification; hardware n=0.**

MEASURED follow-up, 2026-10-23: source
`9ca3aaada7cdfa98b230c6881612d2d6e040812b` additionally requires bystander
GPU progress after the victim channel and its TSG are destroyed, and a supported
asynchronous copy engine (`grce=false`). EXPECTED RED commits `9b0a9140d` and
`5f18f946d` precede those fixes. Python suite: 63 passed/0 failed. Nine Python
mutants have at least two killers; `control-python-mutations.json` records the
current killers and supersedes the seven-mutant counts below. The updated C
probe compiles against the pinned public types and dev headers 7.0.0-38. Actual
post-teardown hardware isolation remains unmeasured.

CODE READ input equivalence, 2026-10-23: Rust/Cargo files are identical between
final source `9ca3aaada` and BF_HEAD `1c78c481e`; the six Rust mutation results
below therefore apply to unchanged Rust inputs. The full suite and Clippy were
also rerun successfully at final source using the dev box and required cargo flock.
Build-farm cleanup currently respects BF_REFUSED/exit75 during Q4's quiet window.

**STATUS: RESEARCH, 2026-10-23 — build-farm validation supersedes the cargo-lock stop; hardware n=0.**

MEASURED build-farm resume, 2026-10-23: the controller authorized CPU builds of
pushed branch heads through the host build farm. At **BF_HEAD
`1c78c481e783b6f2b44a86ed5027a14e5ed7c779`**, all six Rust fault-arm mutants
were killed by at least two tests, with 21/21 tests green before and after the
mutation run and source restored after each mutation. Exact killers, source/mutant SHA256 values, command and
compiler version are in `control-rust-mutations.json`. The deliberately mutated
source is derived by the pushed runner from that BF_HEAD; it is not represented
as an unmodified-head test. No GPU error-control arm or OS guest boot was run. Existing KVM-gated CPU unit
tests are part of the crate suite. The six Rust variants and
seven earlier Python variants total 13 mutants, each with at least two killers.

MEASURED build-farm Clippy, 2026-10-23, same BF_HEAD: kf-chan/kf-qemu/kf-harness,
all targets, optional feature enabled, completed with **277 diagnostic identities
versus the earlier 278, added=0, errors=0**. The removed warning was the new test
crate's missing module documentation. This compiles the native-bystander edits
that were unvalidated at the historical checkpoint below. The full 22-crate
GPU-free suite completed at that same BF_HEAD: **4020 passed, 0 failed, 22 ignored,
236 targets**. Exact package list, commands, counts and Clippy comparison are in
`control-build-farm.json`. The 22 ignored tests were not executed; this result
is not hardware acceptance.

CODE READ supersession, 2026-10-23: `control-checkpoint.json` and the stop text
below preserve the earlier dev-box resource refusal. They are historical; do not
read their “uncompiled” and “unrun” fields as the resumed validation's final state.
The reset decision is unchanged: refusal remains until recovery proof. Full
guest-driver linking against the selected guest kernel and native/OFF/ON hardware
acceptance are still pending; C-object compilation is not that evidence.

**STATUS: RESEARCH, 2026-10-23 — control implemented; final cargo validation blocked; hardware n=0.**

MEASURED stop, 2026-10-23: the first Rust mutant attempt waited 600 seconds for
`/tmp/claude-0/kf-local-cargo.lock`, then flock exited 1 without starting cargo
(0-byte output). This is an unexecuted mutant, not a surviving mutant or failed
Rust test. The mutation runner restored `rc_negative.rs`; the worktree was clean.
Under the lane's “If a step is blocked ... STOP and report” rule, no further
Rust mutations, full suite or final Clippy were attempted. The native-bystander
edits in `6c65d4e21` therefore remain uncompiled/unvalidated here. See
`control-checkpoint.json` for the exact blocked invocation and validation scope.
All owned build targets and guest object/binary artifacts were deleted with
find-delete semantics; no owned cargo/GPU/VM process or resource lock remains.


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

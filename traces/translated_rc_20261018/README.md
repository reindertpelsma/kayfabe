**STATUS: RESEARCH, 2026-10-22.** B1 stopped before implementation.

MEASURED: seed merge `3c7bf3e9d5e6c8999fcc982af787b66ed334df2e` contains the two required
origin sources. `915c1b1de` adds a design and explicit no-op delivery declarations only.
The tests-only EXPECTED RED commit is `31ea54a98c75a1102fdc09f057c75bf54f0d69f5`.

| Check | MEASURED result | Artifact |
|---|---|---|
| Inherited `kf-chan --test translated_ring_rc` | Compiled; 1 pass / 3 fail / 0 ignored | `baseline.log` |
| Extended same target | Compiled; 2 pass / 14 fail / 0 ignored | `expected_red.log` |
| Baseline `clippy -p kf-chan -p kf-qemu --all-targets` | Exit 101; 93 denied errors, six unchanged channel/relay files | `clippy_baseline_errors.json` |
| Fix / green suite / mutants / after-clippy | NOT RUN: implementation stopped at the baseline safety gate | This file |
| GPU/VM/native tests | NOT RUN (n=0; GPU-free lane) | This file |

CODE READ: every cargo command used --release -j2, nice -n 19, the B1-only target and the
required bounded flock. MEASURED: disk free before builds was 28/27/27 GiB, above guard.
Command family (arguments after cargo):
`test -p kf-chan --test translated_ring_rc --release -j2`, and
`clippy -p kf-chan -p kf-qemu --all-targets --release -j2 --message-format=json`.
The two test logs omit dependency-build noise and normalize only the checkout prefix.

MEASURED: Clippy reports 66 arithmetic, 20 indexing and 7 slicing errors. Primary file counts:
`userpump.rs` 51, `userrelay.rs` 3, `userstrand.rs` 4, `usershadow.rs` 9, `chan.rs` 10,
`userread.rs` 16. The messages and primary locations are retained. All six Git blobs exactly
match the required continuation base (`unchanged_baseline.json`); the deny declarations were
not changed. The final clippy delta has NOT been measured; no added-0 or green claim is made.

CODE READ: the lane says STOP when a safety check blocks a step, and forbids relay edits
beyond the narrow hook. Repairing these existing channel/relay lint failures exceeds that hook;
relaxing/suppressing them would bypass the gate. No fix was attempted after that conflict.

Owner options:
1. Recommended: have a separately scoped lane repair the denied relay baseline, then resume
   B1 from this EXPECTED RED checkpoint and run the complete validation/mutation sequence.
2. Explicitly expand B1 to include that baseline repair, with its own tests and mutants.

CODE READ: public-source unprivileged error-context/event proof and the proposed private
record → guest notifier → RC_TRIGGERED path are in `docs/design/V3_TRANSLATED_RC_20261022.md`.
No missing public verb was found for observation. RESET_CHANNEL behavior, native delivery,
ring identity, guest recovery, busy-slot retries and lifetime handling remain UNIMPLEMENTED
or UNVERIFIED. The delivery declarations are stubs, not a working product feature.

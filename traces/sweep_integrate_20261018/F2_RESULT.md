# F-2 verification checkpoint

STATUS: LIVE, 2026-10-19. F-2 verification complete; remaining integration has a separate protected dependency.

Approved production scope is exactly crates/kf-chan/src/grtables.rs and tmode.rs.
EXPECTED RED 08424cf11 contains only the nine tests; 6 pass and 3 fail on old code.
The candidate-4 full release baseline passed 521 tests with 0 failed / 3 ignored.
After the fix, all 9 focused tests pass; the full fixed release suite passes
530 tests with 0 failed / 3 ignored. No existing test was dropped. The fix is
commit 6a32d4e8e, separately following EXPECTED RED commit 08424cf11.

| Mutant | Killing tests | Result |
|---|---:|---|
| Force narrow width for wide classes | 3 | KILLED |
| Widen narrow classes | 2 | KILLED |
| Admit upper bit 25 | 2 | KILLED |
| Drop bound class at production caller | 3 | KILLED |

Exact test names and run-log hashes are in f2_mutations.json; every mutant
returned a runtime test failure, not a compile failure, and the exact fix was
restored. Clippy --release --all-targets: 217 inherited diagnostics before,
217 after, added=0 by package/level/code/message/primary file identity.
Rustfmt is limited to the two approved files. No other kf-chan file, USER path,
channel group, drainer or boot-layer code has been edited. AD104's class stays
8-bit; the Ada 3D and compute rejection cases kill the widening mutant.

Raw build/test logs stay in the lane's own local scratch; bounded counts,
identities and hashes are recorded here. This is GPU-free evidence only.
No rental or owner-host access occurred; current hardware cells remain UNRUN.


The other thirteen prospective integration crates were also tested with their
candidate-4 source files unchanged and the approved F-2 dependency present;
all passed. Their measured counts are in candidate4_baseline_counts.json.
These establish the GPU-free baseline for the next import, not acceptance of
the unimported gpu-types/sweep code. Candidate 5 and the rest of the stack are
not merged yet. The required full integrated AD104 oracles remain UNRUN.

Source/test retention is in f2_test_retention.json. Exactly the approved two
production/test files differ from candidate 4. No other product file changed.
The userrelay family-admission proposal remains unapplied pending scope approval.

H-F2 result: the falsifier did not fire on old code; all three wide-address
method-pair tests were refused there. They all pass after the class-aware fix.

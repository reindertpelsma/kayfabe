# Candidate 6d native harness compile repair

STATUS: LIVE, 2026-10-23. GPU-free repair based on candidate 6d at 70da04ffa.

INFERRED hypothesis before runs: the two native GrConfig literals need the new
CE field, with no implicit seed on GRAPHICS and the allocated host class on
its dedicated-copy ring. FALSIFIER: those are not the actual host ring setups,
the host class does not identify that ring's allocated object, or the repair
still reports E0063. The real setup is read in kf-gr-tier.rs and HostRing.

MEASURED reproduction, 2026-10-23, base 70da04ffa: checking the real
kf-gr-tier binary reports exactly two E0063 missing-ce errors, at the two
native GrConfig literals. EXPECTED RED commit 3c07f8a33 adds six probes:
0 passed, 6 failed by assertion on those compiler diagnostics. No absent target
or missing tool counts as a red test. Fix at 01a171e63: all six pass and the
real kf-gr-tier target checks successfully.

CODE READ repair: the first HostRing is GRAPHICS with real GR objects, so
ce=0 preserves explicit GRCE binding. The second is born on the selected
non-GRCE copy engine, so ce=host2.ce_class() uses that ring's allocated host
object class. HostRing initializes it from the same host RM class selector
used by alloc_ce_object. The production change is exactly two added fields;
no per-die class constant, timing change, or privilege change was added.

MEASURED mutation campaign at commit 01a171e63, 2026-10-23:

| Source mutation | Distinct assertion killers |
|---|---:|
| Seed GRAPHICS from a host CE class | 3 |
| Leave dedicated COPY unseeded | 3 |
| Guess C7B5 for dedicated COPY | 2 |
| Read the other host ring's class | 3 |

[mutants.json](mutants.json) names the killers. Every mutant compiles and
fails by assertion; source is restored byte-for-byte afterward.

CODE READ test scope: the probes compile the actual native initializer
expressions and actual GrConfig field declarations, supplying distinct host
facts through a stand-in getter. They test C5B5/C7B5/C8B5 configuration
semantics and preserve the existing tier/software/video settings. They do not
allocate a physical HostRing or execute work on a GPU. Child compilers are
serialized, inherit the lane's nice level and global Cargo lock, and clean up
their own temporary directories. No assertion was loosened or ignore added.

MEASURED CI diagnosis, checked 2026-10-23: #90's CI run 39772369971 already
caught both E0063 errors in stable Build and aarch64. Its failure was left
unresolved while the earlier local three-crate validation omitted kf-harness.
[ci-history.json](ci-history.json) records the run URL and source identity.

CODE READ history: both literals originate at 01870988bc on 2026-10-07.
The file is byte-identical at #90's base 60655d661 and candidate 6d 70da04ffa;
no other candidate merge introduced or changed it. The required field added
by #90 broke the pre-existing consumer. File absence on the base is refuted.

INFERRED hardware acceptance remains pending. No GPU or guest was started;
Q4 must resume the native/guest gates at the repaired candidate revision.

MEASURED full workspace Clippy --all-targets, 2026-10-23: master at e9c6d39e5
and fix at 01a171e63 each have 532 identical source-bound warning fingerprints;
added 0. [clippy-delta.json](clippy-delta.json) records scope and revisions.
No deny lint or debt ledger was changed. New code lines are at most 100
characters, and rustfmt checks of the changed Rust files pass.

MEASURED hosted CI run 39867898257 at commit 01a171e63, checked 2026-10-23:
stable Build, firmware, broker, and aarch64 pass. The OS-free test step fails
in the unchanged census test: "counted with the log off", left 3 versus 2.
[hosted-ci.json](hosted-ci.json) records the completed run and test counts.
Later stable steps were skipped. This is not a complete CI-green result.

CODE READ the census and raw-device sources are unchanged from candidate 6d.
The test reads a process-global counter before and after its own ioctl while
other raw-device tests can call the same increment path concurrently.
INFERRED cause: concurrent test interference; this run does not establish a
production census defect. No unrelated code or test has been changed.

MEASURED final workspace tests at commit 01a171e63, 2026-10-23:
`cargo test --workspace --no-fail-fast --release --locked -j2` with KVM
forced absent and KAYFABE_SLOW unset: 5754 passed, 0 failed, 21 ignored
across 355 result summaries. This includes every workspace member,
retained crates, binaries, integration tests, and doc tests. The local profile
is the owner-required release profile; hosted stable uses its default profile.
[validation.json](validation.json) records counts, source/log hashes, and
execution settings. No test was loosened or newly ignored.

MEASURED stable-job checks, 2026-10-23: full workspace build and format check
pass; all 14 fast CI gates pass; the recorded Clippy debt gate reports new=0.
The workflow's KVM, SANDBOX, root CAP, and UDMABUF consumer steps all pass.
The CAP build uses the same owner-required release/jobs/lock settings; the
remaining consumer logic is taken directly from the stable workflow. Device
gate counters and their skips remain explicit in validation.json. Namespace
setup/removal uses the workflow script, which leaves host policy unchanged
on this non-GitHub dev box. No GPU or guest was used.

MEASURED final claim-ledger check at commit 01a171e63 plus this evidence copy,
2026-10-23: existing-debt=3756, new=0, resolved=2. Both debt ledgers are
unchanged. All new code and evidence lines are at most 100 characters.

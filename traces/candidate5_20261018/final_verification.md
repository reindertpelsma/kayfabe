# Candidate-5 final GPU-free verification

MEASURED: 3620 passed, 0 failed, 19 ignored across all 22 kf-* crates. No crate was
skipped. The final run follows all three merges, four interaction tests, and the
CUDA source-guard integration correction. All crate Rust sources were touched
before this run to avoid stale CUDA build artifacts. Diagnostics were unset.

MEASURED merge order: gates 560f9bce1, LUT 9ff3dc9e9, bootlayer 0dbcc8a14, each
with --no-ff onto candidate-4. No textual conflicts; one trailing EOF blank in the
rulings document was trimmed. No other branch, master or L5 follow-up was merged.

MEASURED per-crate before/after counts are in crate_counts.tsv: candidate-4 full
3532/1/19 (3524/1/19 excluding the 8 GOP tests), first-merge full 3557/0/19,
LUT-merge full 3561/0/19, first bootlayer attempt 3612/1/19, final 3620/0/19.
The first-merge full run predates two relay-fixture self-tests; subsequent broker
runs include 162 passing tests. The first bootlayer attempt's sole failure was the
obsolete global synchronization source guard; cuda_guard.md records its separate
EXPECTED RED/fix and 4/4/2-test mutant proof. No runtime failure was hidden.
QEMU final 377 includes 356 unit and 21 integration tests; supplied branch counts
that count only unit tests must not be compared directly to these full commands.

MEASURED: candidate-4 memory wait 63117 us exceeded the unchanged 60000 us bound.
Five quiet repeats passed: 6384, 5971, 6992, 5734, 6603 us; sampled one-minute load
2.329–2.444. Under owner Amendment 2 this is a pre-existing LOAD-DEPENDENT flake.
Baseline and final named channel cases pass alone; final peak load 2.843. See the
two isolation JSON files. The previously reported channel failure was not reproduced.

MEASURED: all 35 lock-step mutant anchors match once. Every required form kills
its mutant: 35/35 mutants, 78/78 assertion kills, zero build errors/survivors. The
final full crate run follows mutation restoration. final_mutants.tsv gives every
form's distinct failed-test counts. The owner-approved B progress oracle kills the
carried-credit mutant in 7 tests; stale conn ticks require B/relay, matching A's
documented scope. No mutant was removed.

MEASURED: gate-forced-on/off kills are step 3/3, ring 5/3, C clock 2/2; LUT restart
and retained bindings each fail 2 tests. The full-socket fixture guard fails 2
finite self-tests when removed. Existing instrumentation tests explicitly opt in;
this preserves their enabled-state assertions and tests the production default.

MEASURED: normal and sanitizer broker checks each pass 283 socket checks plus 82
self-tests and 5 clock tests. C units: cursor 66138, geometry 65431, XOR 80752, tick 566237
(candidate-4 tick 566195). XOR 29/29 and tick 28/28 mutants each fail at least two
checks. broker_counts.json preserves the same-command baseline comparison.

MEASURED: comparable clippy 878=878 with no added or removed diagnostics; the
comparison retains a positive control detecting the two actual historical new
warnings. Formatting passes on all changed Rust files. All eight named cursor
source/test blobs equal candidate-4 and their C/Python/Rust checks pass.
CODE READ: inherited strict clippy debt is unchanged; no new lint was suppressed.

CODE READ: interactions.md, cuda_guard.md and rule_audit.md cover copy/reset/boot
choice, refusal counters vs taint, latch binding lifetime, bounded active ring/stats,
and worker-only synchronization. Production diagnostics are optional and off;
no new measurement gate feeds a decision and no new GP_PUT polling path exists.

MEASURED: no hardware/VM benchmark was run, n=0. The 19 standard ignored tests
remain: 9 real broker/root/udmabuf prerequisites, 3 channel open questions, 3 deep
and long display runs, 3 memory debug replays and 1 golden-file generator.
CODE READ: live GPU completion, boot and performance are outside this lane's
GPU-free verification. Host work is build-only; binary metadata follows in the
lane report and READY after all artifacts have been verified.

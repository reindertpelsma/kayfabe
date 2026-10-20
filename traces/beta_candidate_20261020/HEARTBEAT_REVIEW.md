# Nonblocking heartbeat review

STATUS: APPROVED SCOPED IMPORT / GPU-FREE VERIFICATION, 2026-10-20.

CODE READ: approved sweep series32eebb3ab/4e0e0b645/d99c2367b requires the
behavior-preserving f041ceab3 seams. Only unused methods/module are extracted
before the failing-first tests; the old drainer still samples its existing clock
until the green wiring. Source tests16e80e802/39b11e805 plus second-killer3cf12a4b9
are retained. No broad sweep, blocking/clone/command-sequence proposal is imported.

INFERRED falsifiers registered before runs: ignoring fn72 offset, rounding after
subtraction, using missing/invalid metadata, waiting on a GSP/metadata lock,
reusing old-life/fresh-queue clock readiness, or using an ioctl/unbounded clock
read defeats this topic. Each behavior mutant needs at least two named runtime
failures. Clock timing eligibility must remain500ms; no vCPU work or held-lock
wait is added.

CODE READ: intended path is drainer -> try_sample -> GSP try_lock -> current-life
readiness and exact ABI -> SystemInfoCell try_lock -> bounded8-byte offset ->
TimerWindow read_ns -> maximum3 hi/lo/hi mapped rounds. Busy/poisoned state,
missing clock or metadata skips publication. New readiness is cleared by
constructor/reset and fresh queue publication only; console data and scrub
policy are unchanged. GOP-off retains bounded clock metadata, no console seat.

CODE READ mutation harness: standalone rustc -O test harnesses compile the exact
heartbeat/boot-view module source against the same release dependency rlibs;
they avoid relinking the entire QEMU crate per arithmetic mutant. Integrated
cargo tests and the full release regression remain required separately. Compiler
errors never count as kills. All harness outputs stay in lane-local logs.


MEASURED: pre-fix GSP heartbeat subset3 pass/4 fail; exact heartbeat module4
pass/14 fail. After fix GSP56/0 and heartbeat18/0, then one additional independent
Device composition guard was added. All10 mutants are killed by >=2 tests
(7/3/2/2/2/2/2/2/2/2); sources restored. The final19-test module and integrated
cargo regression are recorded separately. No hardware run occurred.

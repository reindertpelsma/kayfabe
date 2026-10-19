# Approved heartbeat result

STATUS: LIVE, 2026-10-19. GPU-free acceptance PASS; hardware UNRUN.

MEASURED: fixture-only94518f95b restores complete QEMU compilation,389/0/0.
Behavior-preserving extractionf041ceab3 preceded tests-only EXPECTED RED16e80e802:
18 runtime failures,5 passes. Helpers32eebb3ab then Device wiring4e0e0b645 made
all23 targeted tests pass. Fresh-queue tests-only EXPECTED RED39b11e805 failed
in both Falcon/FSP paths; fixd99c2367b clears only the clock readiness bit using
the existing fresh-instance decision. The complete25 targeted tests pass.

MEASURED: heartbeat mutations kill5/3/4/2/2/2/2/2/2/2 tests. Actual blocking
GSP and fn72 lock mutants each fail two distinct contention tests. A populated
metadata contention case was strengthened in3cf12a4b9 and rechecked with2 kills.
One parallel stderr line interleaved FAILED with fn72 logging; the authoritative
failures footer proves both runtime failures in the unchanged hashed log.

MEASURED full release at7e9145ff8: QEMU389->407/0/0, GSP54->61/0/0;
all14touched crate suites3338/0/21, no candidate4/candidate5 test removed.
C4QEMU312 and C5QEMU377 are both exceeded. Existing ignored tests remain;
the two extra ignored display data emitters predate this heartbeat topic.
Clippy0new on changed lines,89inherited denied errors (not a stop by ruling);
all-target keep-going raw676 diagnostics, no blanket clean-clippy claim.
Rustfmt81changed files passes plus prior approved userrelay-region receipt.
Later parent-doc/style-only changes and actual rechecks are in phase1_acceptance.json.

CODE READ:500ms unchanged; only borrowed try-lock reads, exact generated fn72
layout decoding, at most9mapped timer loads, scalar arithmetic and existing
shadow stores. No RM clock ioctl, buffer clone, blocking lock, sleep or allocation
on this operation. Busy/poison/missing/invalid state skips publication. Console
cell retention, production scrub, relay, groups/birth and display pacing remain
unchanged. HEARTBEAT_NONBLOCKING.md gives the full path and falsifiers.

LIMIT: no live clock/freshness or family acceptance claim until the rental run.
AD104 also uses this clock path; CPU oracles pass but strict native caps equality
still differs100words. No unchanged owner-host behavior claim; host untouched.

# Combined-candidate hardware regression

**STATUS: RESEARCH, 2026-10-15.** Own box56076314, GA104, open host595.91.07,
Linux guest580.159.04. This topic combines the generated GID inventory repair
with the public C57E semaphore model, for regression on an already passing die.

H-C57E-REG, stated before building/running: the vocabulary-based distinction
between shared C57E semaphores and the newer separate acquire/release model
preserves Ampere fast30 and actual X11 SHM desktop presentation.
Falsifier: any raw arm fails/crashes/is not run; the broker's rendered-pixel
oracle fails; or new guest Xid / flip timeout / an increase beyond the baseline
12 NV assertions appears. Idle STALL diagnostics are retained, not treated as
clean completion. No Turing hardware result follows from an Ampere pass.

The image repair streams are disk/network work, not a second GPU workload.
The candidate build finishes before running timed GPU tests. One GPU job runs
at a time. Windows continues to use its explicit GID-only binary f6d96614.

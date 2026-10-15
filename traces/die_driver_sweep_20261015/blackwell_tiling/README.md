# Blackwell virtual tiling candidate

STATUS: LIVE, 2026-10-15 22:00UTC; GPU-free only, held pending native full-page differential.

Linux580 Xorg sees connected KFB DVI but no valid modes at runtimec2092cf7;
headlessgraphics passes5/5 with10/10 native hashes. Public ogkm580.159.04
b81d58ee0224d1d290bef1c080592b619e184042 nvkms-evo4.c EvoGetCapabilitiesCA /
UsableHwTilesCount / EvoIsModePossibleCA requires usable tiles and a nonempty
required IMP tiling assignment. These were both zero in the authored model.

Hypothesis BEFORE tests: authoring one unscaled virtual tile per head and one
required assignment per active packed head removes mode rejection. Falsifier:
Xorg still rejects modes with correct nonempty tile/assignment output. Native
hardware experiment is UNRUN. Latest W1 steering requires full native caps
oracle equality before another hardware caps experiment, so this candidate is
held, not merged into the running sweep/integration.

Test-first44b9678f8 compiles; two assertions fail0vs1 (before.log). Implementation
sourcea655e1bc9 derives all four ABI layouts, validates bounded head counts,
indices/uniqueness, clears unused output and refuses unsupported DSC. First
post-fix compile caught missing modelcaps input (corrected f4385e7c8), then the
family artifact was stale (first-after.log); emitter adds only T1 CA73 SYS_CAPC
virtual head mask0xf. Full isolated kf-*3251pass/0fail/14ignored,207resultgroups,
exit0at21:57:39UTC,2jobs on trusted CPUhost (after.log). No hardware pass claim.

The native GB206 page contains more tiles/features than this virtual topology;
CPU contracts alone do not meet the new byte-for-byte native-oracle criterion.
No native page is copied into product tables by this candidate. No captured
CCU/CMC replies or unknown CBCB allocation successes were introduced.

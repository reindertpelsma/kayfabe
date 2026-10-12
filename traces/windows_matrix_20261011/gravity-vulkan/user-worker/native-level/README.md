# Public C56F LEVEL: matched native discriminator

STATUS: ANSWERED, 2026-10-12. Native FIFO primitive only; no Windows app verdict.

Exact7d00679ae56a7cf82ab4621a493e42cd37f1a215, RTX4070/595.91.07, unprivileged
uid65534 USER channel/ownVAS/read-only ring, USERD GPU-unmapped. Both matched
pairs PASS: reference vs reference-subroutine, reference-split-held vs
reference-subroutine-split-held. Only firstGP LEVEL bit changes in each pair.
Same source/destination bytes, acquire/real engine copy/WFI/reference and next
MAIN continuation. GP_GET=PUT1 or2, real reference1/copy_matches/CEsemaphore1;
held samples48/no early reference. No hostXid, cleanup successful. START/EXIT,
full per-mode text, binary SHA and exact runner retained. Source construction
GPU-free5PASS; runtime authority never comes from this diagnostic deadline.

Public definition: NVIDIA open-gpu-doc9fdf5c4062007929d9f4e6cbad9c9771fe61b880,
[dev_pbdma.ref.txt](https://github.com/NVIDIA/open-gpu-doc/blob/9fdf5c4062007929d9f4e6cbad9c9771fe61b880/manuals/ampere/ga100/dev_pbdma.ref.txt)
224–232 and770–776: LEVEL controls TOP_LEVEL_GET monitoring, not call/return.
GP_GET commitment404–416 and SYNC_WAIT145–176 do not depend on that monitoring
classification. Public ogkm595.84 clc56f.h273–275 defines bit9 MAIN/SUBROUTINE.

Next patch preserves LEVEL on ordinary and staged spans; synthetic witness uses
SUBROUTINE so private addresses do not replace top-level monitoring. It does not
implement full guest TOP_LEVEL_GET propagation; only GP_GET receipts are relayed.
The GPU-free actual CPU-mapping acceptance case was first RED on strict decoder,
then full six-crate1921PASS/0failed. Windows1560 raw offending entry unavailable;
LEVEL1 remains inferred cause until a matching Windows diagnostic. Added bounded
refusal context ensures any next unsupported descriptor is measured.

Before exact native storage recheck: hypothesis synthetic witness LEVEL1 retains
CONTROL NOP/SYNC_WAIT HOST-consumption ordering. Falsifier storage oracle or
actual fetch/held-prefix order fails, or hostXid. Then exactLinux9+10/fast30/
desktop and one-variable Windows trial of LEVEL acceptance/preservation.

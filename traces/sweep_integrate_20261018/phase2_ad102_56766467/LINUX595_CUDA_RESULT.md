# AD102 guest595 CUDA stopped on filesystem error

STATUS: LIVE, 2026-10-19. MEASURED incomplete at product24906bf33,
open host595.91.07, actual guest595.91.07 verified by NVIDIA-SMI and explicit
profile receipt. SMI_RC=0; first CUDA rung did not produce a workload verdict.
The guest console reports EXT4 htree_dirblock_to_tree inode81233/block33 beyond
i_size135168, journal abort and read-only remount during `find` in the CUP2
build hook. SSH subsequently timed out/reset. No CUDA PASS or product FAIL
is inferred from the interrupted run. Remaining three rungs are UNRUN.
Controller exit0 was caused by explicit termination and is NOT a suite PASS.

H_IMAGE_INFRA stated before the control: candidate4 reaching a clean filesystem
and completed CUDA would falsify the shared-input explanation. The only changed
launch input was QEMU_BIN=.../0452613b/qemu-system-x86_64, whose binary stamp and
hash are recorded separately. Same guest image/profile, source249 launcher and
original CUP2 hook were used. Candidate4 also initializes SMI successfully, then
reproduces the exact inode/block/i_size error and read-only remount. Both runs
were stopped after observation to preserve the lease deadline; neither completed
CUDA. This does not establish which operation originally damaged the image, or
prove that a fresh image would pass. Original cause remains UNRESOLVED. No image
repair, timeout/grader change, or product fix was attempted.

Original logs and export hash receipts are linux595_cuda_* and linux595_c4_*.
The launcher prints its source revision249 alongside the overridden binary;
the authoritative binary stamp in the control driver log is0452613b.
This control is separate from the earlier frozen raw30 R19 comparison.

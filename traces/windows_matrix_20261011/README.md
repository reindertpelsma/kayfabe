# Windows matrix campaign

**STATUS: LIVE, 2026-10-11.** Product candidate `2c917f3e`, following the
OpenGL gears fix. Results are pending; this is not a matrix pass claim.

The current Windows inventory has 108 tests mapping to 65 Linux rows. The native
Windows reference in `traces/windows_baseline_20261010/results.tsv` covers 100:
99 PASS and `gravitymark_vk` TIMEOUT. Eight new YUV overlay tests have no recorded
reference there. Reference instance 55384005 reports a GPU startup error before
Windows becomes available. The owner authorizes replacement rentals and use of
the project SSD; Vast remains untrusted, with no secrets and no unique evidence
left there.

Harness corrections use the independently verified sign-in helper (including
releasing the single-client QGA connection), attach exFAT images directly as USB
mass storage, and validate the checksum belonging to the selected image. The
explicit `WIN_DISABLE_MPO=1` setting records the same DWM policy used by the
passing production gears run. It must not be used to claim overlay support.

Before the first GPU run, hypothesis: these changes permit the existing matrix
supervisor to run CUDA and OpenGL probes on a fresh production guest. Falsifier:
no verified desktop/app disk, or either probe fails its existing GPU/correctness
oracle. GPU results will be recorded whether the hypothesis holds or fails.

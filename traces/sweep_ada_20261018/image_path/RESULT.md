STATUS: LIVE, 2026-10-22. S2 authorized controller plumbing repair.
FALSIFIER BEFORE RUN: selected images reach the real shell QEMU boundary without
explicit image-path propagation. MEASURED: they did not; six assertion failures,
zero errors, 92 existing guard tests green; original abort-tree five also green.
EXPECTED RED test-only commit: 5280961c2.
MEASURED after fix: 98 guard tests + five abort-tree tests = 103 PASS.
Real unchanged fast_suite.sh/run_fast_guest.sh exercise image paths with and without
spaces and a conflicting inherited environment; the QEMU stand-in records argv
and exits unsuccessfully, so this proves launch plumbing, not GPU acceptance.
Provenance checks alone are mocked for the temporary GPU-free script fixture.
CODE READ: controller template explicitly selects the fastguest595 directory it
built; guarded driver requires that option, checks both files before any arm,
and forwards the resolved path through KF_FASTGUEST_DIR. No timeout changes.
MEASURED: manual mutants drop propagation (2 killers), skip image preflight
(3 killers), prefer inherited images (4 killers); zero test errors, restored green.
Reproduce with python3 -B traces/sweep_ada_20261018/image_path/mutate.py.
Historical lease evidence remains unchanged. New lease uses this template with
its newly frozen source revision, own lease id and measured die substituted.

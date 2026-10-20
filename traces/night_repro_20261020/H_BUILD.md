# H build receipt

STATUS: MEASURED / RESEARCH, 2026-10-20 04:26 CEST.

MEASURED source 5c1a864d2116896345b988bd1d6ce5645568d6a8, clean tracked tree. Fresh CPU-only documented build_kf3.sh run at nice 19, Cargo two jobs and Ninja two jobs, isolated source, Cargo target and QEMU source/build/output directories. Built QEMU SHA256 a28e9dbd4cd8707054a547cb9004bdd90f89416214e9b36117bfa44f877b6c6e. New revision output only; pc-bios and qemu-bundle symlinks target the lane-owned qemu-build-kf3. Existing binaries were not replaced. No general product gate or stability claim. Lane tree size at build completion 2,175,637,732 bytes, below 15 GB.

CODE READ: C f8f9d1a5 and H both omit the mirror implementation. H adds superseded to the lifecycle memory summary. Scope-ring overflow has an internal marker; all-PDB logging may depend on existing diagnostic flags. Capture matches without adding flags to only one arm. A missing log match does not prove zero overflow.

MEASURED first H cleanup: durable host archive preserved 18,059,465 appended PM bytes (SHA256 9a979387d9c702bc282e32774f66d25b6d33e7975831e5aa62b7a29d193bf493); guest digest matched before truncation. The pre-run 277,902,209-byte shared prefix was retained. Host CSV gzip readback and tail gzip SHA256 readback passed. No pre-run PM history was deleted. Full harness 65/65; the added owned-tail mutant is killed by two tests.

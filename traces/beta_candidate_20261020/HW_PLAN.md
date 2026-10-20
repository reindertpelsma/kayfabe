# C6 hardware acceptance plan

STATUS: DESIGN-ONLY / WAITING FOR COORDINATOR SLOT, 2026-10-20.

INFERRED primary falsifier, registered before hardware: any Windows effective
compatibility setting differs from the approved profile (with captured caps
replaced as explicitly approved), or results differ beyond matched-run spread,
defeats the no-unintended-behaviour-change claim. No hardware has been run by C6.

CODE READ sequence, only after a coordinator slot; retain exact binary hashes,
host/guest versions, broker version and settings for every arm:

1. v3_gates9/9, plus the reservation gate verdict; then fast suite30/30:
   `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180`.
2. Linux stock-driver CUDA and real compositor/broker retest; compare candidate-5.
3. Windows580.88, declared ABI580.65.06, guest-os=windows and ZERO KF3 env:
   desktop; capture effective-config; compare all P19 migration rows and derived
   caps provenance/unknown notices. Captured whole-page equality is a separate
   diagnostic, not an asserted implementation goal.
4. Heaven/Steel Nomad15-minute loop: zero TDR and zero new host Xid. Matched
   baseline Heaven reference51–57 average; record n and spread, no cherry-picking.
   Record new-content broker fps, 1%/0.1% lows from gaps, gaps>33/>50/>100ms per
   minute/max, repeated presents/sec, guest fps, and capped-run guest/new-content
   agreement within1–2%. A baseline spread needs matched repeated runs.
5. Minecraft Java5minutes; explicitly watch the earlier30-second TDR window.
   Follow with mixed-use aging because the owner observed a separate2-hour failure.
6. New Linux and Windows install walk-through from a clean directory, using only
   INSTALL/RUNNING commands, by someone who did not write them. Verify UEFI/TPM,
   virtio media, GSP setup, identity, read-only doctor, version checks and uninstall.
7. Only if every gate passes: ask the coordinator whether Stage6 may begin.
   No master merge, README overhaul or reviewed PR readiness is claimed now.

CODE READ safety: GPU and VMs are one shared resource; take the documented host
flock only in the assigned slot, with bounded retry. Never stop the owner's VM
unasked. CPU-only build preparation is separate. Store hardware evidence as small
text/PNG; leave large logs on the validation host under the lane's own directory.

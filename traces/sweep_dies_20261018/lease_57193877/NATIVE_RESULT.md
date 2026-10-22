# Ada native checkpoint

STATUS: PARTIAL, 2026-10-22; Windows install in progress.

MEASURED on 2026-10-22, lease 57193877 / rev 170054339, `native_complete/sweep_gates.log`: native gates 9/9, gate10 PASS, twelve USER births.
MEASURED on 2026-10-22, lease 57193877 / rev 170054339, `native_complete/sweep_native_baseline.log`: raw30/30 and CUDA4/4 pass; kernel GR passes in `native_complete/sweep_native_grtier.log`; n=1, spread N/A.
MEASURED on 2026-10-22, lease 57193877 / rev 170054339, `native_complete/native_xid_baseline.json`: exactly five intentional source-matched Xid31 records accepted; every later host-Xid change remains fatal.
CODE READ: no new Linux suite; prior Ada Linux30/30 is recorded under lease57144505 in the inherited evidence.
MEASURED on 2026-10-22, lease57193877, `appdisk_receipt.json`: the 6785255424-byte app disk matches its pinned SHA256 after outbound transfer.
MEASURED on 2026-10-22, lease57193877, `harness_overlay_receipt.json`: only win_vm.sh and matrix.py overlay the pinned checkout from harness rev8cbb7731c. Product crates/qemu/cuda/fastguest sources and compiled source170054339 are unchanged. The receipt records both file hashes and the dirty-file list.
MEASURED on 2026-10-22, lease57193877, `native_complete/sweep_windows_install.log`: LTSC install boot started10:25:00UTC. No Windows acceptance is claimed yet.

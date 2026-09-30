# v3-cdp evidence — CUDA dynamic parallelism in a kf3 guest (2026-09-30)

Design note: `docs/design/V3_CDP.md`. Box: vast 53004208 (`v3060`), RTX 3060 **GA106**, nested KVM,
host and guest driver **580.159.04** (open), app-matrix guest image `guest_apps.qcow2` (16 GiB RAM,
6 vCPUs, no persistence mode). Scripts: `scripts/cdp/` at the revision each job line names. Every guest
run is ONE fresh boot; the kf3 binary's revision is stamped in its `boot.driver.log`
(`kf3-bin-rev:<rev>`). Times are UTC. Files over 16 KiB are gzipped; `qemu.log.zst` is the whole kf3
stderr of the boot.

| dir | what | kf3 binary | result |
|---|---|---|---|
| `bare_metal/host_a` | probe modes 0,4 (both with the ioctl trace), 1, 2, 3, 4:spin, 4:block, `cdpSimpleQuicksort` 128 — 17:55 | none (host GPU) | all OK, `child_ran=1` in every device-launch mode |
| `bare_metal/host_b` | `cdpSimpleQuicksort` 1 000 and 10 000 — 18:13 | none | both `Validating results: OK` |
| `guest_3f67ed95` | master's code: probe 0 then 4 (traced) — 17:56 | `3f67ed95` (= master `a295e2ce`'s code) | mode 0 OK; mode 4 **`child_ran=0`**, stream never complete, watchdog; teardown `kmemsysDoCacheOp … timeout` |
| `guest_exp1_6498628f` | experiment: keep kind `0xF` on the host map + log the leaf — 18:00 | `6498628f` (branch `v3-cdp-exp1`, not for merge) | modes 4 (auto/spin/block), 1, 2, 3 OK; quicksort OK; leaf logged `0x75b470c00000+0x1000 ap=0 at=0x0 kind=0xf` |
| `guest_fix_090b20d9` | the fix: probe 4 (traced), 1, 2, 3, 4:spin, 4:block, 0 (traced); quicksort 128 / 1 000 / 10 000 — 18:13 | `090b20d9` (= fix `46509dce` + scripts) | all OK; `sked=10/0held`; host kernel log empty |
| `guest_fix_090b20d9/app_lane` | `apps_matrix.sh guest cdpfix_090b20d9 cdpSimpleQuicksort` (R3's harness) — 18:17 | `090b20d9` | `verdict=PASS rc=0 secs=7 guest_xid=0` |
| `nvdiff/` | `nvdiff.py diff` of the traces; `uvm_map_dynamic_parallelism_region.txt` decodes UVM ioctl 65 in each | — | noise floor 0; host vs guest lockstep except 2 classified STATUS rows, and at `3f67ed95` 14× `MC_SERVICE_INTERRUPTS` during the hang |
| `jobs/` | the three job logs (`JOB_START … JOB_EXIT`), with the build lines | — | — |

Cross-check that ties the planes together: in each guest run the VA of UVM's
`UVM_MAP_DYNAMIC_PARALLELISM_REGION` (guest userspace, `nvdiff/uvm_…txt`) is the VA of the SKED leaf the
walker reported (kf3, `qemu.log.zst`): `0x75b470c00000` in exp1, `0x768ad2c00000` in the fixed mode-4
run.

⚠ The host kernel log is captured with `journalctl -k --since` (`host_kernel.log`; "-- No entries --"
= nothing logged). `boot.probe.log`'s `HOST_DMESG_LINES=0` is **unmeasured**: `boot_capture.sh`'s
line-count watermark cannot see new lines once the host's dmesg ring is full (`V3_CDP.md` §6).

## Merge bar

Pending at the time this README was first written; the result is appended below by the commit that
adds `merge_check/`.

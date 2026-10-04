# Windows pool-query experiment on the borrowed RTX 4070 PC

**STATUS: RESEARCH, 2026-10-05.** Branch `codex/windows-pool-2026-10-05`.
No successful Windows-through-Kayfabe GPU workload is claimed here yet.

Host: native Linux 7.0.0-31-generic, NVIDIA open 595.91.07, RTX 4070 (AD104).
Windows fixture: existing 580.88 installation with GSP forced, copied from the successful
native reference. The original image is preserved; experiment runs use fresh overlays.
The physical GPU remains attached to the Linux NVIDIA driver. There is no VFIO in this test.

`initial-command.json` and `initial-driver-refusal.log` record the first attempted launch
at `c50fad9ac485f53d45d4ea77a21cb7206267c65a`. QEMU refused to realize the device because
595.91.07 was not a measured host tag. **Windows never booted in that attempt.**
The disk had already passed qemu-img check/convert/check/compare against the native fixture.

The exact 595.91.07 public OGKM tag is `26d82922dc1e444f274294505f73f1c0b8ca682d`.
The driver-matrix measurement uses the repository's four specs, GCC 15.2.0 and pyelftools
0.33. Existing tags are remeasured and compared to the committed ranges before admitting
the new tag; the version check is not bypassed.

Experiment implementation: `ae239848`. The host environment `KF3_GFX_POOL_PROBE=1`
serves only a bounded, invented virtual size query; it is disabled by default.
Two wire-codec tests and three RPC policy tests passed, including malformed lengths,
serialization, poisoned outputs and excessive slot counts. This does not validate the
physical pool formula, initialization, slot lifecycle or context binding.

Runner: `scripts/bench/windows/pc_pool_experiment.py --revision <full SHA> --name <fresh name>`;
add `--pool-probe` for the comparison run. Every run records its command and source revision.
Guest diagnostics: `scripts/bench/windows/pool_probe_status.ps1`, sent over pinned-key SSH
through a controller tunnel. SSH private keys remain on the controller.

See [the design and limits](../../docs/design/V3_WINDOWS_POOL_EXPERIMENT.md) and the native
Linux preemption evidence on research branch `codex/windows-native-trace-2026-10-04`,
commit `82fa680e`. Native mode-1 acceptance is a possible host route, not Windows support.

## Baseline A: query experiment disabled

At product revision `167fe2deaa06fd61f591bc6d2a7c43bd84da0caf`, the exact-driver guard
passes and Windows boots. NVIDIA 580.88 with both firmware registry settings equal to 1
reports Code 43; nvidia-smi exits 9. Kayfabe services 143 RPCs, refuses the pool query,
then observes the teardown and `UnloadingGuestDriver`. No GPU channels are born.
The guest shuts down cleanly and QEMU exits 0 (not a GPU success). Files are in `baseline-a/`.

The host still refuses several GSS legacy controls whose wire layouts have not been
measured on 595.91.07; these guards remain enabled in both experiment arms. A registry
listing emitted an access-denied diagnostic for the class's protected `Properties` key;
the actual NVIDIA class setting and device status were read successfully.

Validation before this run: 1112 ABI/RM tests passed, zero failures. All 124091 compared
cells for the 29 previous matrix tags were unchanged by remeasurement.

# Mode-2 M0 — results: driver probes the emulated GPU, first stall captured

Status: **DONE** (2026-06-03). Companion to [[mode2_plan]] (M0 milestone) and
the spike [[mode2_attestation_spike_GO]].

## What was built

`src/qemu/nvkvm_gpu_emul.c` — a standalone QEMU PCI device `nvkvm-gpu-emul`
presenting a self-consistent GA106 (RTX 3060) identity with the real NVIDIA BAR
layout and a BAR0 register aperture that logs every access and answers the
chip-identity registers.  `scripts/run_mode2_vm.sh` boots a guest (q35/PCIe,
`-snapshot`, behind a pcie-root-port) with the device attached and the BAR0
trace captured via QEMU `-D`.

## Test method (no real GPU needed)

- Host: vast.ai RTX 3060 box. Guest: Ubuntu 24.04, kernel 6.8.0-117.
- Built the **open** kernel modules **575.51.03** in-guest from the 9p-shared
  `open-gpu-kernel-modules` source (tmpfs build).
- `insmod nvidia.ko NVreg_EnableGpuFirmware=1` (after `modprobe ecdh_generic
  ecc` — the open module needs the kernel ECC crypto symbols).
- Triggered `rm_init_adapter` with a tiny C client that opens `/dev/nvidiactl`
  + `/dev/nvidia0` (bypasses the version-matched-userspace requirement that
  blocks the staged 580 `nvidia-smi`).

### Gotcha (cost a few reboots; recorded for the loop)

The Mode-1 guest auto-loads `nvkvm_guest` (Mode-1 forwarding module) at boot,
which registers the `nvidia` device-node namespace and leaves a wedged
`/sys/module/nvidia` kobject (`EEXIST` / `kobject_add_internal failed`) that
blocks `insmod nvidia.ko` until reboot.  Fix: remove `nvkvm` from
`/etc/modules-load.d/` and blacklist it (the Mode-2 VM forwards nothing, so the
Mode-1 module must not load).  `insmod` by path bypasses the `blacklist nvidia`.

## Result

1. **PCI identity is correct.** Guest `lspci`:
   `01:00.0 VGA compatible controller: NVIDIA Corporation GA106 [GeForce RTX
   3060] [10de:2503] (rev a1)`. The open `nvidia.ko` **binds**
   (`Kernel driver in use: nvidia`), `insmod` rc=0.

2. **Chip detection works** (BAR0 reads #0–#7), all answered from the chip
   descriptor:
   - `PMC_BOOT_0` (0x000) → `0x176000a1`
   - `PMC_BOOT_42` (0xA00) → `0x176a1000`
   - `PMC_BOOT_1` (0x004) → `0x0` (VGPU=REAL)

3. **First stall = spike check #1, exactly as predicted.** `rm_init_adapter`
   then spin-polls **one** register 2050× until timeout:
   - off `0x118128` = `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK`
     (the GFW-boot PLM), returning 0.
   - dmesg: `NVRM: gpuIsMsixAllowed_TU102: failed to wait for GFW_BOOT:
     (progress 0x0)` → `No interrupts of any type are available. Cannot use
     this GPU.`

   Trace artifact: `docs/research/mode2_traces/m0_rminitadapter_gfwboot_stall.txt`.

## Exact M1 spec (from kern_gpu_tu102.c `_gpuIsGfwBootCompleted_TU102`)

The driver, before trusting GFW_BOOT, checks the PLM was lowered by FWSEC, then
reads the progress:

1. Read PLM `0x118128`; require `READ_PROTECTION_LEVEL0 (bit 0) == ENABLE`.
   If not, progress=0, keep polling.  → answer with bit0 set (Mode-2 policy:
   PLMs fully unlocked, `0xFFFFFFFF`).
2. Read GFW_BOOT `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05(0)` = `0x00118234`;
   require `PROGRESS (7:0) == COMPLETED (0xFF)`.  → answer `0x000000FF`.

Note `gpuWaitForGfwBootComplete_TU102` first calls `kflcnWaitForHalt_HAL` (waits
for the FWSEC/Booter falcon to halt) but checks GFW_BOOT irrespective of halt
status — so answering the two scratch registers should clear this stall and
expose the next one (falcon/RISCV bring-up, spike checks #2–#5).

Reference chip is GA10x → the driver routes the GFW/MSI-X path through the
`_TU102` HAL (Turing+ shared). Register offsets identical on ga102 swref.

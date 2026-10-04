<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# The first Windows discovery run on kf3: evidence index

These are the evidence files from `docs/design/V3_WINDOWS_DISCOVERY.md`. They were recorded on 2026-10-04
on vast 54071272 (`vwin`): RTX 3060 `10de:2504`, host driver 580.159.04 (open), a nested Broadwell box.
The files are text plus four small PNGs.

These are **not** committed: the Windows images and ISOs, the NVIDIA installer, kernel dumps, the TPM state,
the OVMF variable file, keys, and the Windows password. Those stay on the box under `/workspace/winvm/`,
and the secrets stay in its 0700 directories.

## Layout

| dir | what | kf3 revision |
|---|---|---|
| `preflight/` | sha256 of every medium; NVIDIA 580.88 package facts (INF `DEV_2504`, GSP ELF sections and `.fwversion` `580.65.05`, nvlddmkm strings) | none |
| `provisioning/` | the chain (`provision_box`, `provision_host_driver` 575.51.03 → 580.159.04, `provision_bench_tree`, the Linux 580.65.06 guest) | `494bfeaa` |
| `install/` | `win_vm.sh install`: `install.log` (START/EXIT per step, all three attempts), the check-boot `WIN_*` lines, the VM's Secure Boot vars at install and after Windows ran, `swtpm_setup.log` (public EK only), the first-logon transcript, QEMU command lines, QMP events, screenshots | `1c479f30` (no kf3 device) |
| `basic1/` | stage 1: Windows on kf3 with the signed GOP, no NVIDIA driver. Two boots across a guest reboot (TPM persistence), `WIN_*` checks, Windows evidence (`collect.ps1`), screenshots | `b98bdbec` |
| `nv1/` | stage 2a: `nv-install` (`gsp_on.ps1` output), then nvlddmkm's first start. fn 1 is refused; Code 43 | `b98bdbec` |
| `nv2/` | stage 2b: the same disk with C2. RM init runs 141 commands, then tears down after `GR_GFX_POOL_QUERY_SIZE`; Code 43. Windows evidence, nvidia-smi, BitLocker | `f429b2be` |
| `linux_baseline/` | Linux guest 580.65.06 on the same box and flags (`display=on,gop=on`), `KF3_RPC_TRACE=1`: the display lane and the CUDA ladder (cup2, cup3, cup8 PASS) | `b98bdbec` |
| `windows_vs_linux.md` | `scripts/bench/windows/rpc_diff.py nv2 vs linux_baseline`, with control names from the ogkm 580 SDK headers | none |

## Notes on the files

- **kf3 logs.** `*_qemu.trimmed.log` keep every line except the repeated status heartbeat: one heartbeat in
  30 and the last one are kept. Each file's first line says how many were dropped.
- **Windows evidence.** Text normalized to UTF-8 with LF: BOM removed, control bytes replaced by `?`.

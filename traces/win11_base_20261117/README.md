# traces/win11_base_20261117: the new blank Windows 11 + NVIDIA base image (build and offline clone proofs)

**Run:** one unattended build of a new proprietary Windows 11 Pro base image on the pc (AMD Ryzen 9 7900, RTX 4070), 2026-11-17 23:24Z to 2026-11-18 00:16Z, followed by five throwaway offline clone boots of the sealed result.
**kf3 revision:** `origin/next` `6bfa2dbc2` (contains PR #438, the Windows profile as the default), kf3 QEMU 10.2.4 built on the pc (`kf3-bins/6bfa2dbc`), display broker built from the same revision (headless test backend). **Host driver:** NVIDIA 595.91.07, Linux 7.0.0-34. **Guest:** Windows 11 Pro 25H2 build 26200.8037 (unactivated), NVIDIA 580.88 driver. swtpm 0.10.1; distro QEMU 10.2.1 for the setup boot.
**What it shows and the numbers:** [`docs/design/V3_WINDOWS_BASE_IMAGE.md`](../../docs/design/V3_WINDOWS_BASE_IMAGE.md), section 8.

Text only. No identity value is in these files: the seed, the SMBIOS UUID and serials, the MAC, the disk serial, the GPU UUID, the slirp subnet, the loopback ssh port, MachineGuid, the TPM EK hash, the computer name, the guest password and key are replaced by `<field>` placeholders (`GPU-<uuid>`), argv files are masked, and no checksum of the private image is here. No PNG, no `state.json`, no TPM or OVMF blob, no NVIDIA or Microsoft binary.

| Path | What it is |
|---|---|
| `build/progress.log`, `prepare.out`, `install_attach.out`, `install_screens.log`, `post.out` | the build steps in order; `install_screens.log` is the OCR text of each distinct screen of the Windows installer (the "Product key" page is the only one that needed an answer) |
| `build/gpu.log`, `gpu_result.json`, `gpu_kf3_env.json` | the GPU stage: SSH up, driver install, enable once, bounded poll, power and GPU repairs, restart test; `kf3_env` is the whole kf3 environment (`KF3_RPC_TRACE=1` only) |
| `build/gpu_check.txt`, `gpu_check_after_restart.txt`, `gpu_nvidia_smi_1.txt`, `gpu_nvidia_smi_2.txt` | problem code and `nvidia-smi` after the first enable and after an in-guest restart |
| `build/gpu_registry_1.json`, `gpu_registry_2.json`, `gpu_power1.json`, `gpu_power2.json`, `facts_after_restart.json` | registry and power readback, Windows facts |
| `build/gpu_census.json`, `gpu_windows_profile_lines.txt`, `gpu_dmesg_new_filtered.txt` | the kf3 census of the GPU stage (Xid 0), the Windows-profile log lines (first 80), the new host kernel-log lines |
| `build/seal.out` | the seal: host-side TRIM, convert, zero check (0 non-zero bytes in the unused clusters) |
| `clones/clone1`, `clones/clone2` | two offline throwaway clones of the sealed image through `propimage_check.py --apps none`: `h__result.md` (the rows), `extra.json` (network negative test, problem code, `nvidia-smi` fields, clock offset, event log), `host_nvidia_smi_query.txt` (the same fields on the host), `registry.json`, `census.json`, `argv_masked.txt` |
| `clones/abl_plain`, `abl_noguestdriver`, `abl_uuidauto` | ablations of the kf3 device properties on the same clone: neither `guest-driver=` nor `gpu-uuid=` (what a plain `win_vm.sh run` builds), no `guest-driver=`, no `gpu-uuid=` |
| `scripts/` | the scratch driver scripts that ran on the pc (adapted from the image-B scratch scripts); they call the repository's `propimage.py`, `propimage_check.py`, `gsp_on.ps1`, `desktop_prep.ps1`, `gate_enable.ps1` and `kf3_census.py` unchanged except `propimage_check.py --apps none` (PR #481) |

Reading the clone `END` files: `rc=1` is the `hold` row (`released`), not a failed check; every check row of clone 1 and clone 2 is PASS or OK. The three ablation clones are expected to differ: see section 8.3 of the design note.

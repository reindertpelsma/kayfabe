# V3 — what is intentionally NOT in git

**STATUS: LIVE, 2026-10-10.** Rule (owner, 2026-10-10): all code and scripts of the kayfabe work are in git; only
regeneratable or re-downloadable things stay out. This is the list of what stays out, where it lives, how to get
it back, and whether losing it would lose information. The host-only scripts that *were* only on the Windows
bench host are now in `scripts/bench/windows/host_tools/` (README there). Host paths are on the trusted bench
host under `/var/lib/kf-windows-20261005` (`$W`) unless noted; the host's address is deliberately not recorded here.

## Categories left out

| category | where | size | regenerate / re-download | loses information if lost? |
|---|---|---|---|---|
| `kf3-bins` (QEMU+kf3 binaries per revision, 167 revs) | `$W/kf3-bins/<rev>/`; sha256 of the pre-ablation set in `$W/kf3-bins-sha256-before-ablation-20261009.txt` | 15 GB | `scripts/bench/build_kf3.sh` at the revision (rev is the dir name; QEMU 10.2.4 source below). A bench claim names its revision, so any binary is rebuildable from git. | No (binary equals source@rev), except that rebuilt binaries are not bit-identical to the measured ones; the recorded sha256 are the measured ones. |
| Windows baseline image | `$W/baseline/windows.qcow2` (+ `OVMF_VARS.fd`, `command.json` = the exact QEMU argv, built 2026-10-05 with kf3 `c50fad9a`); `$W/baseline-a/` (115 MB test copy) | 21 GB | `scripts/bench/windows/win_vm.sh install` (unattended Windows 11 Enterprise LTSC 2024 evaluation, build 26100, Secure Boot with the kayfabe db cert, swtpm TPM, virtio-win 0.1.302, Win32-OpenSSH, NVIDIA 580.88 driver). Media URLs and sha256 are pinned in `win_vm.sh` (ISO sha256 `67cec586...5da68`, 511,285,043 bytes, Microsoft fwlink). About an hour on a nested box. See `scripts/bench/windows/README.md`. | No for content; the install is reproducible. The 90-day evaluation clock and the TPM state differ per install. |
| Windows run overlays / desktop disks | `$W/windows-desktop/windows.qcow2` (17.8 GB, interactive owner disk; `windows.qcow2.corrupt-20261008` 17.3 GB is a corrupted copy), per-run overlays under `$W/winprod/runN/`, `$W/pti-win/`, `/srv/win-storage/{windows.base,windows.vars,windows.rom,data.img,setup.img}` | tens of GB | Overlays are created from the baseline by the runners; the desktop disk is the baseline plus the owner's sign-in. `windows-desktop/secrets/` holds the throwaway guest password and an SSH key (never copied anywhere). | No (guest state only). The sign-in must be redone (`KF_GUEST_PW`). |
| `kfapps.iso` and app-matrix downloads | `$W/appmatrix/image/kfapps.iso` (6,785,255,424 bytes, sha256 `183c13aa6b2c3c20475f7e1c5eed9979ca973056886d2b519c013386ba4da652`, label KFAPPS), `$W/appmatrix/dl/` (blender, ffmpeg, furmark2, heaven, llama_cuda, qwen_gguf, hashcat, vkpeak, ...), `appmatrix/pipdl`, `appmatrix/built/*.exe` | 6.6 GB (iso), 247 MB dl | `scripts/bench/windows/appmatrix/build_appdisk.sh` (reads `apps.json`; per-file sha256 in the image `manifest.json`, also at `$W/appmatrix/manifest.json`); the `built/` test programs come from `appmatrix/build_tools.sh` and `cup2/cup3/cup8` sources in the repo. | No: every item is pinned by URL and sha256 in `apps.json`. |
| Dumps | `$W/dumps/` (`run223-hang.elf`, `run232-hang.elf`, `bc260.raw`, `*WATCHDOG*.dmp`, ...), `$W/guestlogs/`, `$W/winprod/runN/forensics`, ETL/EVTX | 125 GB | Not regenerable per se: they are captures of specific hung runs. The findings drawn from them are in `traces/windows_tdr_hunt_20261010/README.md` and the hang root-cause docs; the Volatility outputs that mattered are quoted there. | **Yes, the raw dumps are the primary evidence** of runs 223, 226-232, 237, 260. Keep them on the host (or cold storage) as long as the hang claims are cited. |
| Volatility venv and Microsoft symbols | `$W/dumpcfg/venv` (75 MB), `$W/re/pdb/{dxgkrnl,dxgmms2,win32kbase}.{pdb,json,sys}` (19 MB; Microsoft images and PDBs), ntoskrnl symbols in the volatility cache | 100 MB | Venv: recipe in `host_tools/README.md`. Symbols: fetched from the Microsoft symbol server by the PDB GUID and age read from the image/dump (Volatility 3 `pdbconv` produces the JSON); the images come from the Windows guest. The plugins and `symz*.py` are in git. | No. |
| QEMU and ogkm source trees, build trees | `$W/qemu-10.2.4/` + `qemu-10.2.4.tar.xz` (135 MB), `$W/qemu-build-kf3/` (298 MB), `$W/win-qemu/` (4.6 GB), `$W/ogkm-595/` (154 MB), `/workspace/ogkm-tars/595.84.tar.gz` and `open-gpu-kernel-modules-595.84/` (155 MB), `third_party/` submodules | ~5 GB | QEMU 10.2.4 from qemu.org (tarball + the patch/device in `qemu/hw/misc/kf3/`); NVIDIA open-gpu-kernel-modules 595.84 from `github.com/NVIDIA/open-gpu-kernel-modules` tag `595.84`; register offsets are generated from it by `tools/derive_*.sh`. | No. |
| Cargo `target/` dirs | `$W/target*`, `ceint-target` (2 GB), `kf-int-target`, `pti-target`, `rawclient-target`, per-worktree `target/` | several GB each | `cargo build` at the pinned toolchain (`rust-toolchain.toml`). | No. |
| Vast.ai templates and box state | the vast "Ubuntu 22.04 VM" template with `vms_enabled=true` is chosen at rent time (see `scripts/bench/box/README.md`); boxes are retired by own ids only | n/a | `scripts/bench/box/provision_full.sh`, `scripts/bench/provision_*.sh`; `vast_reaper.sh` for teardown. API keys are never stored in the repo. | No. |
| Guest OS images | `/opt/nvkvm-guest/*.qcow2` (Ubuntu 24.04, Mint 22.3, SteamOS), `/srv/iso/`, docker volumes (`nvkvm-steamos-state*`, 160 GB), `/srv/win-storage/win11x64.iso` (8.5 GB) | hundreds of GB | Distribution ISOs / images re-downloaded from their vendors; the provisioning scripts are in `scripts/bench/provision_guest_*.sh` and the `nvkvm-rs` repo. | No, except SteamOS state volumes (user data of earlier experiments). |
| Logs, `*.state`, `wr-launch-N.log`, `winprod/runN/` timelines | `$W` | small | Not regenerable. Run summaries that carry claims are in `traces/` and `docs/`. | Partly: per-run raw logs beyond what is quoted in `traces/` are lost if the host is wiped. |
| Git bundles of hunt branches | `$W/tdrhunt/*.bundle` | few MB | The branches are on origin (`claude/tdr-hunt-20261010`, `claude/tdr-opus-20261010`). | No. |
| Python venv `__pycache__`, `*.pyc`, caches | everywhere | small | Regenerated automatically. | No. |

## Closed-driver analysis files: kept on the host, names only

Standing rule: reverse engineering of the closed NVIDIA driver is for diagnosis only; nothing derived from it and
no reference to it goes in the repo. Under `$W/re/`: `fn.py` (PE function disassembler helper), `ghidra.zip`,
`ghidra_12.1.4_PUBLIC/`, `img/` and `img2/` (module images carved from a memory dump). Also `$W/tdropus/vdis.py`
(disassembler helper marked "never committed"). Their outputs, if any, are not in the repo. Nothing else on the
host is known to contain decompilation, strings or offsets of the closed driver; `tdr-run*.sh` and `rm_instloc.ps1`
mention the service name `nvlddmkm` only as a Windows service/ETW filter name.

## Secrets (never in git)

`$W/windows-desktop/secrets/win_password` and its SSH key, the throwaway guest passwords (now `KF_GUEST_PW` and
`KF_GUEST_SSH_PW` in the committed runners), vast API keys. One committed instance existed: the throwaway test-guest password was in a command line in
`docs/design/V3_HELD_HOLE_HANDOFF_20261010.md` (commit `f99caf2b`, on the branches that contain it). It is scrubbed
in the tree as of this commit; history is not rewritten (the value is a throwaway guest password, with no access
outside the bench guest). Rotate it the next time the Windows guest image is rebuilt.

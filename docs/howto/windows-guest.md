# Windows guest setup: stock driver, a few registry settings

**STATUS: LIVE, 2026-10-17.** What a Windows 11 guest needs to run NVIDIA's stock driver on kayfabe. Labels: [measured] = a run on named hardware, [inferred] = reasoning.
Source of the measurements: `docs/design/V3_WINDOWS11_STOCK_INSTALL_20261013.md` (branch `claude/default-route-evidence-20261013`, not merged yet), `traces/win11_userring_20261014/`.

## What was tested
- **Windows 11 Pro, build 26200 (the full retail edition)** [measured, 2026-10-14, `traces/win11_userring_20261014/`]: desktop, the app matrix in `traces/win11_userring_20261014/` (the benchmark output names "Microsoft Windows 11 Pro", kernel 10.0.26200).
- **Windows 11 Enterprise LTSC 2024, build 26100** (Microsoft's 90-day evaluation) [measured]: the image `scripts/bench/windows/win_vm.sh install` builds, and the one used on rented boxes.
- **Driver:** NVIDIA's stock Windows driver (580.88), installed with NVIDIA's own installer or INF, unmodified. No driver, kernel or Windows binary is patched.

## The settings it needs (and why)
The stock GeForce driver on Windows runs its resource manager on the CPU by default. Kayfabe serves only the GSP firmware path, so the driver has to be told to use GSP.
Without the first setting the driver stops with **Code 43** [measured on Windows 11 and on LTSC; source: `docs/WINDOWS_GUEST_SETUP.md`, 2026-10-17]. All values are DWORDs and take effect after a restart.

| # | Key | Value | Why |
|---|---|---|---|
| 1 | `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` | `1` | forces GSP; `<index>` is the display adapter's own key (`0000` on Windows 11 Pro, `0001` on the LTSC image: read it, never assume) |
| 2 | `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | `1` | the same flag; the service key alone is not enough, set both |
| 3 | `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | `5` | without it, opening the Start menu reset the display driver on Windows 11 Pro (cause not proven; source: `docs/WINDOWS_GUEST_SETUP.md`, 2026-10-17) |

Read the display adapter's own registry key before setting its `EnableGpuFirmware` value; do not assume the adapter index. Apply all three DWORD settings above and restart Windows. These are settings for an existing development guest, not a complete installation guide.

`scripts/bench/windows/gsp_on.ps1` applies the two GSP settings while installing the driver (the operation used by `win_vm.sh nv-install`). Setting them before the driver's first start avoids Code 43.

## Linux guests
Nothing like this: the Linux guest uses NVIDIA's stock driver (the `.run` installer or the distribution package) unmodified, with no registry-style settings and no guest patches.

## Other things a Windows guest needs (not settings)
- A unique SMBIOS UUID, serial and MAC per VM, and offline setup until after OOBE (Windows Autopilot once matched a default-identity VM to an unrelated company tenant); an explicit `gpu-uuid`.
- Do not snapshot an image with a Windows update staged (it crash-loops `dwm.exe`); block updates in test images.
- Windows sleep/wake is not supported yet.

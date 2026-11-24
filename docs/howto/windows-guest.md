# Run a Windows guest

**Assumes:** a Linux x86-64 host with KVM, an NVIDIA GPU from Turing onward driven by the open NVIDIA kernel module, and QEMU 10.2.4 with the kayfabe device, and a Windows 11 guest (Pro build 26200 or Enterprise LTSC 2024 build 26100) with NVIDIA's 580.88 driver.

A Windows 11 guest runs NVIDIA's stock driver on kayfabe. You install Windows and the driver yourself; kayfabe adds the emulated GPU and needs a few registry settings in the guest, which one script sets. <!-- src: docs/WINDOWS_GUEST_SETUP.md (STATUS: LIVE, 2026-10-17) -->

## The steps

1. **Install Windows 11** in a VM that has the kayfabe device ([Install](installing.md), [Command-line reference](cli-reference.md), [Attach to a VM you already have](attach-to-existing-vm.md)). Give the VM a TPM 2.0 and UEFI Secure Boot firmware if you want what the tested guests had; a guest without them has no recorded acceptance run.
2. **Run `kayfabe-windows-setup.bat`** in the guest before NVIDIA's driver installer, and again after it ([below](#the-settings-it-needs-and-why)).
3. **Install NVIDIA's driver** with NVIDIA's own installer. Version 580.88 is the Windows driver that has been tested.
4. **Restart** the guest. Check `nvidia-smi` and that Device Manager shows no error on the display adapter.
5. Give a Windows guest `display=on,gop=on` on the device (with `-vga none`), even when nobody watches the screen; view it with a broker window or VNC ([Display and the broker](display.md)). Without the virtual display, NVIDIA's Windows driver resets itself (System log: `nvlddmkm` event 153) within two minutes of boot, and in one series the guest then powered itself off. Secure Boot guests need the boot display driver signed and given with `gop-efi=`. <!-- src: traces/fill_probe_20261124/README.md (Experiment 2) -->


You supply and license Windows yourself. Kayfabe ships no Windows and no NVIDIA binary ([Credits and notices](credits.md)).

## What was tested

- **Windows 11 Pro, build 26200** (the retail edition): the desktop and the application matrix on an RTX 4070. <!-- src: docs/WINDOWS_GUEST_SETUP.md, README.md -->
- **Windows 11 Enterprise LTSC 2024, build 26100** (Microsoft's 90-day evaluation): the image the test harness builds, used on rented hosts. <!-- src: docs/WINDOWS_GUEST_SETUP.md, scripts/bench/windows/README.md -->
- **Driver:** NVIDIA's stock Windows driver, version 580.88, installed with NVIDIA's own installer or INF, unmodified. No driver, kernel or Windows binary is patched. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- Applications run unmodified from the vendors' complete installers; the recorded results are on [What works today](https://kayfabe.dev/works-today/) and in the repository README. <!-- src: site/content/works-today.md, README.md "What runs today" -->
- GPU families: see [Requirements](requirements.md).

## The settings it needs (and why)

The stock GeForce driver on Windows runs its resource manager on the CPU by default. Kayfabe serves only the GSP firmware path, so the driver has to be told to use GSP. Without the first setting the driver stops with **Code 43**. This was seen on Windows 11 Pro and on Windows 11 Enterprise LTSC. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->

One script sets all three: `scripts/windows/kayfabe-windows-setup.ps1`, with a wrapper `scripts/windows/kayfabe-windows-setup.bat` that starts it with the execution policy it needs. Copy both files into the guest and run it from an elevated prompt (right-click, Run as administrator), **before** NVIDIA's driver installer so that the display is not lost after the install:

```
kayfabe-windows-setup.bat            REM before the NVIDIA installer: sets what can be set (phase pre)
REM ... run NVIDIA's installer, then:
kayfabe-windows-setup.bat            REM after it: finds the adapter's key, sets the rest (phase post)
shutdown /r /t 0                     REM the values take effect after a restart
```

All values are DWORDs. The script prints each value it sets and reads it back, changes nothing that is already right (run it as often as you like), refuses to run without administrator rights, and never restarts the machine. It does not need the NVIDIA driver to be installed. <!-- src: scripts/windows/kayfabe-windows-setup.ps1 -->

| # | Key | Value | Phase | Why |
|---|---|---|---|---|
| 1 | `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` | `1` | post | forces GSP; `<index>` is the display adapter's own key (`0000` on Windows 11 Pro, `0001` on the LTSC image: the script reads it, never assumes it) |
| 2 | `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | `1` | pre | the same flag; the service key alone is not enough, set both |
| 3 | `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | `5` | pre | without it, opening the Start menu reset the display driver on Windows 11 Pro (the cause is not known) |

<!-- src: scripts/windows/kayfabe-windows-setup.json (this table is generated from it and a test keeps them equal) -->

Why two phases: the display adapter's key (row 1) only exists once a driver is bound to the NVIDIA device, so it cannot be set before the install. Phase `pre` sets rows 2 and 3 (creating the service key if it is missing); phase `post` finds the adapter's key from the NVIDIA device's driver key, sets row 1 and repeats the others. The default, `-Phase auto`, picks `post` when an NVIDIA device is bound to NVIDIA's driver (Microsoft's Basic Display driver does not count) and `pre` otherwise. If the display shows Code 43 after the install, run the script again and restart.

Options (they follow the file name): `-Phase auto|pre|post`; `-DriverKey "{4d36e968-e325-11ce-bfc1-08002be10318}\0001"` to name the adapter's key instead of discovering it; `-InstallDriver <dir>` for a single step, where `<dir>` is the folder NVIDIA's package was extracted to (it holds `Display.Driver\nv_dispi.inf`): it sets the `pre` values, disables the device, adds and installs the INF, sets row 1, reads everything back and re-enables the device. If a value does not read back it leaves the device disabled and says how to enable it. Exit code 0 is success, 1 a value that did not read back, 2 a refusal (not elevated, bad arguments), 3 no NVIDIA device or no NVIDIA driver bound where one is needed. <!-- src: scripts/windows/kayfabe-windows-setup.ps1 -->

`scripts/bench/windows/gsp_on.ps1` does rows 1 and 2 for the harness (`win_vm.sh nv-install` runs it). <!-- src: scripts/bench/windows/gsp_on.ps1 -->

These are settings for a development guest, not a complete installation guide for a production system.

Not verified end to end: `kayfabe-windows-setup.ps1` has run only against a fake registry and a Windows runner's registry, not before a real NVIDIA driver install; rows 1 and 2 are the part that `gsp_on.ps1` applies and reads back in the test harness.

## Windows needs no environment variables

The Windows support set (the behaviours the Windows driver needs and Linux does not use) is on by default. It is selected by what the guest announces about itself when its driver starts, which names the driver build. That announcement comes from the guest, so it is not a security boundary: an honest Linux guest never reaches the Windows set, but any guest that announces Windows driver 580.88 does. No path into another VM or the host was found in a reading of the code; a dedicated hostile-guest review has not been done. On a host that runs only Linux guests, or several tenants, set `windows-support=off` on the device. A Windows guest on such a VM is refused by name and is expected to end in Code 43; that has not been tested. Some of the answers are made up by kayfabe rather than taken from the GPU (pool sizes, and success for the preemption and zcull binds with nothing bound on the host); the Windows driver needs them and they apply to a declared Windows guest only. Running Windows with no environment file has not been run on hardware yet. <!-- src: docs/design/V3_WINDOWS_SUPPORT_PROFILE.md, docs/OWNER_RULINGS.md section AT -->

## Other things a Windows guest needs

- **A unique identity per VM.** Give each VM its own SMBIOS UUID, serial number and MAC address, and do the setup offline until after the first-run wizard (Windows Autopilot once matched a default-identity VM to an unrelated company tenant). Pass `-uuid`, or `vm-id=`, so that the GPU UUID the guest sees (`gpu-uuid=auto`) is stable: without either it is random at every boot. <!-- src: docs/WINDOWS_GUEST_SETUP.md, crates/kf-rm/src/gpuuid.rs, scripts/bench/windows/win_vm.sh -->
- **Do not snapshot an image with a Windows update staged**: it crash-loops `dwm.exe`. Block updates in test images. <!-- src: docs/WINDOWS_GUEST_SETUP.md -->
- The guest changes its own Secure Boot variables (it adds the 2023 CAs to `db` and updates `dbx`). The variables file is live state of that VM: never regenerate it or copy it to another VM. <!-- src: scripts/bench/windows/README.md "Known limits" -->
- A failed NVIDIA driver slows everything down: with the driver in Code 43, boot to a login took about 20 minutes and the guest ignored a shutdown request for 5 minutes. Fix the settings first. <!-- src: scripts/bench/windows/README.md "Known limits" -->

## Known limits

- **Windows sleep and wake is not supported yet.** One optional QEMU patch addresses the wake hang of a guest that has Hyper-V enlightenments; it is applied only when you ask and is not part of any release binary ([Building from source](building.md#optional-qemu-patch-sleep-and-wake-of-a-windows-guest)). <!-- src: docs/WINDOWS_GUEST_SETUP.md, README.md, docs/howto/building.md -->
- Edge can reset the display driver after 7 to 16 minutes of video playback. <!-- src: README.md "Not finished" -->
- Windows 11 reached a passing Windows gate on these GPUs: TU106, TU116, GA102, GA104, GA106, AD102, AD104, GB205 and GB206. GB202 was intermittent: one failure, then two passes on one machine. On GB203 (RTX 5070 Ti) the gate passed once; the screen had stayed black before the display fixes. These are results of past runs, not a promise for the current source. See [Requirements](requirements.md). <!-- src: traces/die_coverage_20261104/{turing,ampere,ada,blackwell}/README.md; traces/ampere_hw_20261103/README.md; traces/blackwell_win_20261103/ -->
- Heavy scenes can reset the display driver (a TDR), and on some cards it loops. See [Troubleshooting](troubleshooting.md).

## Not documented yet

- A Windows install by hand from a plain ISO on a machine without the test harness. [Attach to a VM you already have](attach-to-existing-vm.md) covers the QEMU side, not tested.
- Windows 10, or other Windows 11 builds than the two above.
- A successful guest run with another NVIDIA driver version. Windows driver 581.51 was attempted on an AD102 card and failed with Code 43; the same board's native Windows run passed.

To install and boot a throwaway Windows 11 guest with the project's own script, see [The Windows test harness](windows-harness.md).

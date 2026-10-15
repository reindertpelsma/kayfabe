# V3: a stock Windows 11 install under kayfabe — what it needs, and how it was found

**STATUS: LIVE, 2026-10-13 (measured on the owner's full Windows 11 build 26200 install, host RTX 4070, kayfabe `bf510bdf`, NVIDIA Windows driver 580.88; `nvidia-smi` and the desktop work after the two guest settings below).**
Related: `THE_WINDOWS_AXIS.md` (why a stock GeForce driver is monolithic on Windows), `V3_WINDOWS_DISCOVERY.md` (the `EnableGpuFirmware` discovery on Ampere), `scripts/bench/windows/gsp_on.ps1`.

## Result

A fresh Windows 11 (full, not LTSC) installed under kayfabe, with the stock NVIDIA 580.88 driver, came up **Code 43** and, after the fix, runs the desktop with the GPU in WDDM mode (`nvidia-smi`: RTX 4070, driver 580.88, CUDA 13.0, Start menu, Explorer, Search and Edge listed as GPU processes). It needs two guest-side settings that the LTSC baseline already carried:

1. **`EnableGpuFirmware = 1` (DWORD) in BOTH** the display adapter's class key `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` **and** `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm`. `<index>` is the adapter's own driver key, read from `DEVPKEY_Device_Driver` (it was `\0000` on this install and `\0001` on the LTSC baseline: never assume it; a value written to the wrong index does nothing and leaves a stray key). The service-key value alone did NOT fix it (measured: it was set and the device was still Code 43). Without the flag the stock GeForce driver runs the monolithic RM, which kayfabe (a GSP client only) cannot serve.
2. **`HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode = 5` (DWORD)**, the overlay policy every automated run already sets. After the GSP fix and without it, opening the Start menu reset the display driver (GSP `Running -> Suspending -> Halted -> ... -> Running`, black screen: a lit head with no scanout window). With it set (and a reboot) the desktop stays up. ⊘ Causation is NOT proven: it is the one known difference from the baseline, and the symptom disappeared with it; the matrix has always run with it.

Both take effect only at the next driver start (restart Windows).

## Evidence (measured, in order)

- The installed driver was the same package as the baseline (`nv_dispi.inf_amd64_fe5f369669db2f36`, `32.0.15.8088`); `guest-driver=580.65.06` label unchanged.
- The Windows 11 driver took a different path from its FIRST register access (BAR0 trace, same tracer as the working run): it read display register `0x610018`, wrote `0xc80` to seven registers at `0xd96c + n*0x50`, wrote `0x88080` twice, and stopped (9 writes, kayfabe stuck at `phase=Cold`). The working LTSC run starts with `0x100c40`, `0x100c10`, `0x88080` and continues into the GSP boot. Deterministic, identical after restarts.
- NOT the VM configuration: on the LTSC baseline the same QEMU configuration (12 GB RAM, unique `-uuid`/`-smbios`, explicit `gpu-uuid`, new MAC), each delta alone and all together, gave Code 0 and `phase=Running`.
- NOT the CPU model: on the Windows 11 disk, SVM hidden, no Hyper-V enlightenments, both, and the original all failed identically.
- Reproduced on the LTSC baseline with a guest agent in about 3 minutes: remove the NVIDIA `HKLM\SOFTWARE\NVIDIA Corporation` and ProgramData state, `pnputil /remove-device` the GPU, remove the service-key flag, reboot: **Code 43**, `phase=Cold`. Setting the flag in the (re-created) class key and the service key and rebooting: **Code 0**, `phase=Running`. (The first-start state, not the OS build, was the difference.)
- The stock driver's default is documented in `THE_WINDOWS_AXIS.md`: registry default `0x12` (`MODE_DEFAULT | ALLOW_FALLBACK`); the Windows default policy depends on the SKU (device id + subsystem id), workstation/server SKUs and TCC/MCDM; for a GeForce in WDDM it ends monolithic. `1` means "force, no fallback".

## Setting it up by hand (until a guest-setup helper exists)

In an elevated Command Prompt (do not rely on the Start menu before step 3):
1. Read the class key: `powershell -NoProfile -Command "(Get-PnpDeviceProperty -InstanceId (Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'PCI\VEN_10DE*').InstanceId -KeyName DEVPKEY_Device_Driver).Data"`.
2. `reg add "HKLM\SYSTEM\CurrentControlSet\Control\Class\<that key>" /v EnableGpuFirmware /t REG_DWORD /d 1 /f` and `reg add "HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm" /v EnableGpuFirmware /t REG_DWORD /d 1 /f`.
3. `reg add "HKLM\SOFTWARE\Microsoft\Windows\Dwm" /v OverlayTestMode /t REG_DWORD /d 5 /f`, then `shutdown /r /t 0`.
`kf-gsp-on.cmd` (tools disc, `tools/windows-guest/kf-gsp-on.cmd`) does steps 1-2 by reading the driver key from Windows.

## Other findings from setting this VM up (each measured)

- **Windows Autopilot matched a default-identity VM to an unrelated company tenant** during OOBE once the guest had internet (profile cached in `ServiceState\wmansvc\AutopilotDDSZTDFile.json`; forced enrolment, sign-in required). A fresh install must have a unique, persistent SMBIOS UUID/serial and MAC, and should stay offline until OOBE is done. Nothing was signed in or enrolled.
- **`gpu-uuid=auto` with an explicit VM `-uuid` is refused** at host 595.91.07: the host-ABI gate refuses `GPU_GET_GID_INFO` (`0x2080014a`), so the per-VM GPU UUID cannot be derived; use an explicit `gpu-uuid=<uuid>` until the ABI row exists.
- **An image snapshotted with a staged Windows update fails on first boot** (servicing commit fails, `dwm.exe` crash loop). Snapshot only with no update pending and updates blocked.
- Device reset and driver unload were exercised on this install (restart, hard reset, disable/enable on the LTSC baseline): see `V3_DEVICE_RESET.md` §11.

## Open product question (owner)

A stock Windows 11 user who installs the stock driver gets Code 43 until the flag is set. Options: (A) a one-time guest helper or installer that sets it before/after the driver install (what this page does by hand); (B) present a workstation/server-class identity so the driver's default turns GSP on (changes the device and subsystem ids the guest sees: conflicts with the host-derived identity rule); (C) check whether newer Windows drivers default GSP on for Ada (unmeasured). Recommendation: A, documented, with the helper on a guest-tools disc.

STATUS: LIVE, 2026-11-18 (built and measured on the pc 2026-11-17/18 with kf3 `6bfa2dbc2`; the hypotheses and falsifiers in section 6 were written BEFORE the runs and the results are in section 8; open items in section 9).

# The new blank Windows 11 + NVIDIA base image (the layer under later proprietary applications)

Owner instruction, 2026-11-17 (verbatim intent): "first construct a Windows 11 proprietary image from the win11 ISO on the PC (blank), this time also with NVIDIA installed, working, under kayfabe broker, nvidia-smi responding in guest; do not forget registry keys, and I hope the env flags required to launch Windows have been fixed." The first application layer on it is Ansys Student (later, not part of this work).

Labels: MEASURED = a command run on the pc on 2026-11-17/18 (named, evidence in `traces/` or the coordinator evidence directory); CODE READ = read in a named file, not run; INFERRED = reasoning; DESIGN INTENT = decided, not built. The repository holds no Windows, NVIDIA or other proprietary binary and no identity value of the image; the sealed image is private (never in git, R2 release assets or the public site).

## 1. What is built, in one paragraph

A fresh Windows 11 Pro 25H2 (build 26200) installed unattended from the official multi-edition ISO (edition index 6, no product key, not activated, local administrator, offline), with the Red Hat virtio drivers, OpenSSH (one way in: `setup_ssh.ps1`), autologon, NVIDIA 580.88 installed on the kf3 device with `EnableGpuFirmware` forced before nvlddmkm's first start, the repo's own power repairs applied once, and the registry list in section 5. Sealed the way image B was (host-side NTFS free-space zeroing on a throwaway overlay, `qemu-img convert -c`, manifest, read-only). It is a NEW identity: a new random private seed, so every hardware id, the MachineGuid, the TPM, the OVMF variables and the computer name differ from images A and B. It is built with the repository's own propimage tooling (`propimage.py`, `propimage_check.py`, `gsp_on.ps1`, `desktop_prep.ps1`, `gate_enable.ps1`, the answer file template, the kf3 flags sample) strung together by the scratch driver scripts that actually ran, kept in [`traces/win11_base_20261117/scripts/`](../../traces/win11_base_20261117/scripts/) (adapted from the image-B scratch scripts, which were never committed; folding them into one tested driver is a follow-up). Two small gaps found on the way are separate PRs: `propimage_check.py --apps none` (#481) and `propimage.py tools --unattended` (#482).

## 2. Launch flags and environment a Windows guest needs TODAY (answers the PR #431 question)

CODE READ on `origin/next` `5e24c253c` (kf3 binary under test: `6bfa2dbc2`, which contains PR #438; the only later commit touching `crates/` is a test and a capacity table in `kf-cuda`):

- **Environment: none.** `OWNER_RULINGS.md` section AT hardwired the 18 profile flags (they act per guest declaration of the Windows driver at fn 1; `windows-support=on` is the device default). `scripts/bench/windows/kf3_flags.sample.env` now holds only `KF3_RPC_TRACE=1`, a diagnostic that adds one log line per answered command and that the census reads; it changes no behaviour. `KF3_WIN_HIDE_UNTABLED_VIDEO` is off and stays off.
- **Device properties** that `propimage.py build_argv` passes for every kf3 mode (the harness already does what PR #431 wants `win_vm.sh run` to do): `gpu-uuid=<seed-derived>` (not `auto`), `guest-driver=580.65.06` (the wire layout of the Windows 580.88 driver, `state["qemu"]["guest_driver"]`), `fb-mb=4096`, `bar1-size=134217728`, `bar2-size=33554432`, and for the broker display `display=on,gop=on,gop-efi=<signed boot GOP>,display-broker=<socket>,display-broker-uid=<uid>`. Machine: `pc-q35-10.2,accel=kvm,smm=on`, memory-backend-memfd (kf3 needs shared guest RAM), Secure Boot OVMF, swtpm CRB, virtio-blk with a seed-derived serial, `qemu-xhci` + `usb-tablet` + `usb-kbd`, `-vga none`.
- **`win_vm.sh run` on `next` (without PR #431):** CODE READ: no default for `--guest-driver` (unset means the property is not passed), no default `gpu-uuid` (unset means `auto`, a hash of `-uuid`, random per QEMU start without `WINVM_VM_IDENTITY`), no env file. PR #431 (open, CI red on `stable` and `freshness`) measured on 2026-11-15, BEFORE PR #438 merged, that a plain run (no env file, no `--guest-driver`, no gpu-uuid) ended in Code 43 (GSP Running -> Suspending -> Halted). Which of the three (env file, guest-driver, gpu-uuid) mattered was NOT bisected there; the env file is gone since #438. **MEASURED here (section 8.3): `guest-driver=580.65.06` is still required, the env file is not, `gpu-uuid=` is not needed for Code 0 but fixes the identity.**
- Guest side, for Code 0: `EnableGpuFirmware=1` in the display class key of every present NVIDIA display function and in `Services\nvlddmkm`, written BEFORE the first start of nvlddmkm on the device (section 5).

## 3. The unattended install recipe (what the build does, in order)

1. `prepare`: refuse to run twice; check the Windows ISO against its pin (sha256 `76898470...e465e3`, build 26200, 8,471,603,200 B, on the pc at `/srv/win-storage/win11x64.iso`); copy the virtio-win 0.1.302 ISO (pin `303f7ae4...67949d`) and the OpenSSH MSI (pin `ddec9c53...b7d9`) from the existing images' media (read only, hash-checked); download the NVIDIA 580.88 installer from NVIDIA's official host and check the repo pin (`90c49f92...3f63a9`, 890,807,536 B); draw a random private seed and refuse any whose derived ids or ssh port collide with images A or B or a listening port; `propimage.init_state`; a fresh ed25519 key pair; render the answer file with a new computer name and two deviations (below); build the KFTOOLS CD; fresh swtpm state (`swtpm_setup`, as root); the Microsoft-keys OVMF variables; a 160 GB thin qcow2.
2. `install`: swtpm and the distro QEMU 10.2.1 (the kf3 build has no GTK front end) in `setup` mode, headless (`-display none`, QMP screendump), `restrict=on` (the build guest is never online), `nice 19`. A key press only in the first 40 s ("Press any key to boot from CD"). The Windows Setup "Product key" page is not skipped by the answer file (MEASURED on image B): it is answered from the screen text (OCR of a QMP screendump), Tab to "I don't have a product key" and Enter, never by timing. No product key is entered; Windows stays unactivated (licence status 5), exactly as images A and B.
3. `post`: probe with `propimage.GUEST_PS`, facts, `Dwm OverlayTestMode=5`, autologon (`desktop_prep.ps1`, password on stdin only), restart test, quiesce, record MachineGuid, TPM EK hash, CPU fingerprint and the guest's own clock, clean shutdown.
4. `fw`: enrol the public boot-GOP signing certificate into the image's OVMF variables (`virt-fw-vars --add-db`, after `sbverify` shows the GOP is signed by it).
5. `gpu` (kf3 + the display broker, the GPU lock held): INF route (`gsp_on.ps1 -NoEnable`) -> autologon -> restart -> enable the device once with a signed-in desktop (`gate_enable.ps1`) -> bounded poll for Code 0 and the seed GPU UUID -> the repo's own `power_fix` and `gpu_fix` once -> registry readback -> delete `C:\kf` -> in-guest restart test -> record -> clean stop.
6. `seal`: host-side TRIM (throwaway overlay on `qemu-nbd --discard=unmap --detect-zeroes=unmap`, `ntfswipe`), `qemu-img convert -c`, state, manifest, zero check, read-only.
7. Two OFFLINE throwaway clones of the sealed image with `propimage_check.py --apps none` (restrict=on, recorded clock): power, GPU repair and identity rows, then a held guest for the extra checks (nvidia-smi, problem code, network negative test, registry readback, clock window), then the census of the kf3 log.

Answer-file deviations from the repository template (both from image B, both needed because nobody sits at the console and the guest has no network): `BypassNRO=1` in the specialize pass (OOBE finishes without a network) and a `FirstLogonCommands` entry that runs the SAME `setup_ssh.ps1` from the KFTOOLS CD (still the one way SSH gets in).

## 4. Identity and clock (derived, never captured)

CODE READ in `propimage.py`: one seed; every id is `sha256("kayfabe-propimage/v1\0<seed>\0<field>")`: SMBIOS UUID and the three serials, MAC, disk serial, GPU UUID (`kf3-gpu gpu-uuid=`), the slirp /24 (inside `10.16.0.0/12`, never QEMU's default), the loopback ssh forward port, and a clock gap of 600..3599 s. The guest address inside the slirp /24 is assigned by slirp's DHCP; the guest has no route out (`restrict=on`, only the ssh hostfwd on 127.0.0.1): the sealed clone needs no network and uses none. The guest throwaway password is `derive_password(seed)`; with a private random seed it is private, and it is stored (as autologon `DefaultPassword`) only inside the private image. The computer name is `KFW-<8 hex>` from the seed hash and is not stored in `state.json` (the schema has no field for it). What cannot be derived (MachineGuid, the TPM EK, the CPU fingerprint) is recorded from the guest once and compared on every run. The clock: install and build boots run the real clock (offline, so the owner's rule section AU gives a WARN, not a refusal); the guest's own UTC at the last probe is recorded as `last_time_at_stop_utc`; every `offline`/`ci` boot starts at that time plus the seed gap, never earlier, with `clock=vm`. No software floor applies (a proprietary image uses exactly the clock of its `state.json`, section AU item 3).

## 5. Registry keys and guest settings, and where each comes from

| Setting | Value | Why | Source |
|---|---|---|---|
| `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\<index>` `EnableGpuFirmware` (every present NVIDIA display function) | 1 (DWORD) | forces GSP; without it Code 43; must exist before nvlddmkm's first start on the device | `gsp_on.ps1`, `docs/howto/windows-guest.md` setting 1, `propimage_check.render_gpu_fix` (per boot, new instance) |
| `HKLM\SYSTEM\CurrentControlSet\Services\nvlddmkm` `EnableGpuFirmware` | 1 | the same flag, both are needed | `gsp_on.ps1`, howto setting 2 |
| `HKLM\SOFTWARE\Microsoft\Windows\Dwm` `OverlayTestMode` | 5 | the Start menu reset the display driver without it (cause unknown) | `desktop_prep.ps1`, howto setting 3 |
| Winlogon `AutoAdminLogon=1`, `DefaultUserName`, `DefaultPassword`, `DefaultDomainName='.'`, no `AutoLogonCount`; `PasswordLess\Device\DevicePasswordLessBuildVersion=0` | as stated | a signed-in desktop at every boot (the device is enabled once with a desktop; app checks need one) | `desktop_prep.ps1` |
| `HKLM\SOFTWARE\OpenSSH` `DefaultShell` = Windows PowerShell, sshd Automatic, TCP 22 rule, `administrators_authorized_keys` (the VM's public key only) | as stated | the one transport the harness uses | `setup_ssh.ps1` (`propimage.py write_tools_dir`) |
| `HKLM\SYSTEM\Setup\LabConfig` `BypassCPUCheck=1` (setup only) | 1 | the Zen 1 CPU profile is not on Windows 11's list; TPM, Secure Boot and RAM checks stay on | answer file, windowsPE |
| `BitLocker\PreventDeviceEncryption=1` and the specialize component | 1 | no device encryption (owner, 2026-10-04) | answer file |
| `WindowsUpdate\AU NoAutoUpdate=1`, `WindowsUpdate ExcludeWUDriversInQualityUpdate=1`, `DriverSearching SearchOrderConfig=0` | 1, 1, 0 | no update traffic, and never a Windows Update driver for the kf3 device | answer file |
| `DeviceGuard EnableVirtualizationBasedSecurity=0`, `...\HypervisorEnforcedCodeIntegrity Enabled=0` | 0 | no VBS or HVCI | answer file |
| `Session Manager\Power HiberbootEnabled=0`, hibernation off | 0 | a resumed kernel must not meet a fresh kf3 | answer file, `power_fix` |
| `CrashControl CrashDumpEnabled=2`, `DisplayParameters=1` | 2, 1 | kernel dumps and bugcheck parameters on screen | answer file |
| `CI\Policy VerifiedAndReputablePolicyState=0` | 0 | Smart App Control off (it blocks the unsigned CUDA probes) | answer file |
| `OOBE BypassNRO=1` (build only) | 1 | OOBE without a network | deviation B1 |
| Power plan: sleep, hibernate, unattended sleep, hybrid sleep, monitor, disk timeouts 0 on AC and DC; wake sign-in off; machine inactivity limit 0; screensaver off | 0 | an awake, visible desktop for unattended checks | `propimage_check.power_fix` (per boot; applied once here so a first clone boot changes nothing) |
| NOT set: `HwSchMode`, `TdrLevel`, `TdrDelay` | absent | diagnostic knobs of earlier experiments, never part of a measured profile | `hwsch_off.ps1`, `tdr_off_etw_arm.ps1` (read-only evidence here) |

## 6. Hypotheses and falsifiers (stated before the runs)

| # | Hypothesis | Falsifier (the result that proves it wrong) |
|---|---|---|
| H1 | The unattended install reaches SSH offline with the answer file plus the one product-key answer, no human | Setup stops on a page other than "Product key" (the screens log names it), or SSH is not up 40 min after start |
| H2 | On `next` with PR #438 and NO env file (only the diagnostic RPC trace) the NVIDIA 580.88 driver installed on the kf3 device reaches problem code 0 and `nvidia-smi` prints the seed GPU UUID | Code 43 or `nvidia-smi` failing after the first enable; or no `WINDOWS PROFILE` line in the kf3 log (the profile did not engage) |
| H3 | `guest-driver=580.65.06` and the seed `gpu-uuid=` are NOT needed on a plain run any more (the profile re-selects at fn 1); only the env file was the missing piece of the 2026-11-15 failure | An offline clone started without `guest-driver=` (or with `gpu-uuid=auto`) reaches Code 43 or a different GPU UUID (this one is an ablation boot, section 8) |
| H4 | The GPU is Code 0 again after an in-guest restart inside the same kf3 QEMU without any repair | After the restart the device is not Code 0 within 240 s, or `nvidia-smi` shows no GPU (then `gpu_fix` is needed, recorded) |
| H5 | The kf3 census of the GPU stage has no Xid and no DEAD channel; unserviced and refused counts are small and stable between the two clone boots | any `NVRM: Xid` line, any DEAD channel, or an unserviced/refused count that differs by more than 10 percent between clone boots |
| H6 | `nvidia-smi` prints a number for every field the host returns (name, driver, memory total, temperature, clocks) and `[N/A]` only for fields kf3 does not serve (utilisation, MEASURED `[N/A]` on image B) | a field printed `[N/A]` that the host `nvidia-smi` prints as a number and that image B printed as a number |
| H7 | After the seal, two offline throwaway clones (recorded clock, `restrict=on`) both pass power, gpu and verify rows; the guest has no route out; the clock sits in `[base, base+1800 s]` | any row fails, the guest reaches a public address, or the clock is outside the window |
| H8 | The seal's zero check finds no non-zero byte in any unused NTFS cluster, and the sealed disk is 10 to 25 GB (a blank Windows plus driver, compressed) | non-zero bytes in unused clusters, or a size above 40 GB |

## 7. PC rules this work obeys

The pc is opportunistic: every stage is one job under `flock /root/prov/pc_gpu.lock` with START/END lines in `/root/prov/PC_ACTIVE.log`; only own PIDs are killed; no VFIO and no GPU unbind or bind (kf3 sessions only); at least 60 GB stays free; images A and B, ownerwin, `/data`, the coordinator backup and the Ansys download directory are never written; the pc is never rebooted; every result file (text only) is copied to the coordinator as it is produced; the sealed image lives on the pc only until it is backed up.

## 8. Results (MEASURED on the pc, 2026-11-17 23:24Z to 2026-11-18 00:16Z)

Setup: AMD Ryzen 9 7900 pc, RTX 4070 (AD104), host driver 595.91.07, Linux 7.0.0-34, kf3 QEMU 10.2.4 from `origin/next` `6bfa2dbc2` (contains #438; the pc build dir `kf3-bins/6bfa2dbc`), display broker built from the same revision (headless `--backend test`: nothing appeared on the owner's monitor), distro QEMU 10.2.1 for the setup boot, swtpm 0.10.1. Evidence (text only, identity values and checksums stripped): [`traces/win11_base_20261117/`](../../traces/win11_base_20261117/README.md); the full set is also on the coordinator in `/workspace/bench/win11_base_20261117/`.

### 8.1 What was built

| Item | Value (MEASURED) |
|---|---|
| OS | Windows 11 Pro 25H2, build 26200.8037, retail channel, **not activated** (licence status 5), no product key entered (as images A and B), UTC, en-US, Secure Boot on, TPM present (swtpm CRB), BitLocker fully decrypted and off, 0 problem devices after install |
| Guest user | `kfimage` (local administrator, autologon); password derived from the private seed |
| Drivers | virtio-win 0.1.302 (storage, net, serial, rng, pvpanic, fwcfg, smbus; no balloon), NVIDIA 580.88 (INF 32.0.15.8088, `nvlddmkm.sys` signature Valid, signer NVIDIA Corporation) installed by `gsp_on.ps1 -NoEnable` then enabled once with a signed-in desktop |
| SSH | OpenSSH 10.0.0.0, sshd Automatic, Windows PowerShell as default shell, the image's own public key only |
| Sealed disk | 11,101,075,456 B (10.3 GiB allocated, 160 GiB virtual, qcow2 compressed), `C:` used about 24 GB before the driver; the free space of the NTFS volume (148,777,988,096 B) is all zero (0 non-zero bytes, checked on the sealed copy) |
| Identity | new random private seed, so every hardware id, MachineGuid, the TPM EK, the OVMF variables, the ssh port and the computer name differ from images A and B (the seed was checked against both before use); CPU profile `kayfabe-amd-v1` (AMD host required, as for A and B) |
| Where | pc `/workspace/pc1/winvm/win11_base/sealed/20261117T234916Z/` (read-only `image.qcow2`, `state.json`, `manifest.json`); sha256 and size in the pc's `SEALED_IMAGE.txt` and the coordinator's private note, not in the repository (the image is private; `state.json` holds the seed, the key pair and the TPM and OVMF blobs) |

### 8.2 Times

| Step | Wall time | Note |
|---|---|---|
| `prepare` (media check, NVIDIA download from NVIDIA's official host, seed, state, tools CD, swtpm, disk) | 46 s | the 890 MB installer matched the repo pin; the ISO matched its pin |
| `install` (QEMU start to SSH up) | 9 min 31 s | headless, offline, one on-screen answer: the "Product key" page, Tab then Enter on the first attempt, found by OCR of the screen text |
| `post` (facts, DWM key, autologon, restart test, record, clean stop) | 2 min 4 s | autologon survived the restart |
| `fw` (boot-GOP certificate into the OVMF variables) | under 1 s | db grew by one certificate (6133 to 7272 bytes) |
| `gpu` (kf3 + broker, driver install, enable, power and GPU repairs, restart test, record) | 6 min 38 s | SSH up 24 s after the QEMU start, driver install 66 s, Code 0 3 s after the first enable |
| `seal` (NTFS free-space zeroing, `qemu-img convert -c`, manifest, zero check) | 9 min 26 s | the convert alone 191 s |
| One offline clone check | about 3 min each | including the held guest and the evidence copy |
| Whole build, `prepare` to sealed | 34 min 30 s | far under the 60 to 70 GB, 6 to 9 h estimate for image D, which has four application installs and downloads; this is the blank layer under it |

### 8.3 The hypotheses of section 6, with the outcome

| # | Outcome | Evidence |
|---|---|---|
| H1 unattended install reaches SSH offline | **HELD** | SSH up 9 min 31 s after start; the only page that needed an answer was "Product key"; `build/install_screens.log` |
| H2 Code 0 and the seed UUID with no env file | **HELD** | first enable: problem code 0, `nvidia-smi` printed the RTX 4070, 580.88 and the seed UUID after 3 s; 212 `WINDOWS PROFILE` / `FABRICATED answer` lines in the kf3 log; environment was only `KF3_RPC_TRACE=1` (`build/gpu_result.json`, `build/gpu_check.txt`) |
| H3 `guest-driver=` and `gpu-uuid=` not needed | **FALSIFIED for `guest-driver=`, held for `gpu-uuid=` (as to Code 0)** | on offline clones of the sealed image: *plain* (neither property, what `win_vm.sh run` builds on `next` today): Code 43 (`CM_PROB_FAILED_POST_START`), `nvidia-smi` cannot communicate with the driver, 0 profile lines; *no guest-driver*: the harness's `nvidia-smi` query did not answer in 120 s (host log: GSP booted, 4 RPCs serviced, 0 profile lines); *uuid-auto* (no `gpu-uuid=`, `guest-driver=` kept): Code 0, `nvidia-smi` works, 110 profile lines, but the GPU UUID is the auto one, not the seed's (the identity row is INFO, the image's identity check would block it). Cause, CODE READ in `kf-qemu/src/device.rs`: an unset `guest-driver=` defaults to the HOST's driver version (595.91.07 here), whose wire layouts are not those of the Windows 580.88 driver (wire 580.65.06), and the Windows profile needs the host-stub cell of that wire (`V3_WINDOWS_SUPPORT_PROFILE.md`, *Narrowing noted*) (`clones/abl_*`) |
| H4 Code 0 after an in-guest restart, no repair | **HELD** | after `Restart-Computer` inside the same kf3 QEMU the desktop signed in and the device was Code 0 with the seed UUID within 3 s, `gpu_fix` not needed (`build/gpu_check_after_restart.txt`) |
| H5 census: no Xid, no DEAD channel, stable counts | **HELD** | GPU stage (driver install and three guest boots in one QEMU log): Xid 0, unserviced 945 (all code 76), refused 497; clone 1: unserviced 449, refused 190; clone 2: unserviced 449, refused 202 (6 percent apart); host dmesg: two `unknown NVRM ioctl command: 0xf0` lines per kf3 session, nothing else (`clones/clone*/census.json`) |
| H6 `nvidia-smi` numbers wherever the host has one | **FALSIFIED** | the guest prints the name, driver, memory (4096 MiB total, the `fb-mb` window), PCIe generation and WDDM, but `[N/A]` or `0` for temperature (0 C; host 41 C), fan (`[N/A]`; host 58 percent), power draw and limit (`[N/A]`; host 36 W, 200 W), clocks (0 MHz; host 2490 and 10251 MHz), utilisation (`[N/A]`; host 0 percent) and P-state (`[N/A]`; host P2). Utilisation is the part open PR #459 (owner decision section AV) addresses; the others are not covered by it (`clones/clone1/extra.json`, `host_nvidia_smi_query.txt`) |
| H7 two offline clones pass | **HELD, 2 of 2** | `propimage_check.py --apps none`: power OK, gpu PASS (seed UUID exact), verify PASS (SMBIOS, serials, MAC, disk, GPU UUID, MachineGuid, clock window 1800 s, CPU fingerprint, TPM, EK hash all MATCH); no route out (TCP 443, DNS and HTTP all fail, no default route); problem code 0, 0 problem devices; guest clock 130 and 133 s after the recorded base (base = recorded stop time plus the seed gap, never earlier than the stop time); no Kernel-Power 41, no minidump. The run's exit code is 1 only because the `hold` row's verdict is `released` (see 8.6) |
| H8 zero check and size | **HELD** | 0 non-zero bytes in the unused clusters; 11.1 GB |

### 8.4 The launch flags and environment, answered (the PR #431 question)

| Item | Needed today? | Evidence |
|---|---|---|
| `KF3_*` environment / env file | **No.** Only `KF3_RPC_TRACE=1` was set, a diagnostic that adds log lines (the census reads them) | H2: Code 0 with only that variable |
| `windows-support=on` | default, nothing to pass | `OWNER_RULINGS.md` section AT; the profile engaged (212 and 110 lines) |
| `guest-driver=580.65.06` | **Yes.** Without it the Windows 580.88 guest fails (Code 43 or a hang) | H3 |
| `gpu-uuid=<fixed>` | Not for Code 0. **Yes for a sealed image**: otherwise the guest sees a different GPU UUID (a hash of `-uuid`, random per start without `WINVM_VM_IDENTITY`) | H3 `uuid-auto` |
| `fb-mb`, `bar1-size`, `bar2-size`, `display=on,gop=on,gop-efi,display-broker,display-broker-uid` | as `propimage.py build_argv` builds them (`fb-mb=4096` for these images) | CODE READ, and every boot above |

The exact command is `propimage.py build_argv` for modes `install` and `ci`; its masked output for the clone boots is `traces/win11_base_20261117/clones/clone1/argv_masked.txt`.

**PR #431 (`win_vm.sh run: the Windows kf3 defaults`): status and what is still needed.** It is open, its merge state is DIRTY and CI is red (`stable`, `freshness`). Its premise, measured 2026-11-15 before #438, was that three things were missing: the env file, `--guest-driver` and `gpu-uuid`. After #438: the env file is obsolete (its 18 flags are hardwired and setting them "does nothing now", `kf3_flags.sample.env`), so the part of #431 that loads `kf3_windows_defaults.env` should be dropped; the `--guest-driver 580.65.06` default is **required** (this section), and the per-VM `gpu-uuid` default is needed for a stable GPU UUID but not for Code 0. A plain `win_vm.sh run` on `next` today (dry run, CODE READ) builds `kf3-gpu,fb-mb=8192,bar1-size=...,bar2-size=...,display=on,gop=on,gop-efi=...,id=kf0` with no `guest-driver=` and no `gpu-uuid=`, which is the *plain* ablation above: it ends in Code 43. So "the env flags required to launch Windows" are fixed (none are required), but a plain `win_vm.sh run` is **not** fixed until `guest-driver` is defaulted. The propimage harness is not affected: it always passes both properties.

### 8.5 Registry and guest settings read back from the clone (MEASURED, both clones identical)

Read inside the throwaway clone (`clones/clone*/registry.json`): the NVIDIA display class key (`0001`) `EnableGpuFirmware=1` and `Services\nvlddmkm` `EnableGpuFirmware=1` (the class key `0000` is the leftover Microsoft Basic Display Adapter and carries none); `Dwm OverlayTestMode=5`; Winlogon `AutoAdminLogon=1`, user and password set, `DefaultDomainName='.'`, no `AutoLogonCount`; `DevicePasswordLessBuildVersion=0`; OpenSSH `DefaultShell` Windows PowerShell; `NoAutoUpdate=1`, `ExcludeWUDriversInQualityUpdate=1`, `SearchOrderConfig=0`, `PreventDeviceEncryption=1`, VBS 0, HVCI 0, `HiberbootEnabled=0`, `CrashDumpEnabled=2`, `DisplayParameters=1`, `VerifiedAndReputablePolicyState=0`, `BypassNRO=1`; `HwSchMode`, `TdrLevel`, `TdrDelay` absent (not set, section 5); no `hiberfil.sys`; `C:\kf` absent; sshd Running and Automatic; Windows Update service Manual. The power plan and screensaver values were applied once in the build by the harness's own `power_fix` (power_fix reported OK in both the build and the clones, so a first clone boot changes nothing) (`build/gpu_power1.json`, `gpu_power2.json`).

### 8.6 Harness findings

- `propimage_check.py` could not check an image that has no application (`--apps` took only known names): **PR #481**, tests first, `--apps none`.
- The repository's answer file assumes the owner runs `setup_ssh.ps1` at the console and that OOBE has a network; both builds patched the rendered file in a scratch script: **PR #482**, `propimage.py tools --unattended`, tests first and three new mutants (70 of 70 killed).
- The Windows Setup "Product key" page is not skipped by the answer file (image B measured it too; no key is entered). The build answers it from the screen text (OCR) with Tab and Enter; a UI-skipping answer-file element would need a key or a decision, so it is left as it was.
- `propimage_check.py --hold-until-file` ends with the `hold` row verdict `released`, and the run's return code is 1 for it although every check row passed. Not fixed here (a manual-iteration aid, nothing depends on it); noted so nobody reads `rc=1` as a failed check.
- Operating notes from the scratch driver (not repo bugs): `pkill -f` and `pgrep -f` match their own `ssh` command line when the pattern is spelled on it; a background job started over `ssh` needs `ssh -f` and redirected descriptors or the call does not return.

### 8.7 Rulings checked

- **Clock rule (section AU):** build boots ran the real clock offline (a WARN, not a refusal); the clones ran the recorded clock offline (`restrict=on`); the recorded stop time is the guest's own UTC at the last probe before the clean stop, and every clone booted at that time plus the seed gap (3599 s here), never earlier. No software floor (a proprietary image uses exactly the clock of its state, item 3).
- **Pins (section AW):** the media pins (ISO, virtio-win, OpenSSH MSI, NVIDIA installer) are supply-chain integrity and were enforced; the NVIDIA installer was fetched from NVIDIA's official host and compared with the repo pin (route 3).
- **"Nothing from NVIDIA is stored in our images, R2 or release assets" (`docs/DOWNLOADS.md`, 2026-11-10, in the paragraph on Linux apps on a Worker box):** images A and B hold the INSTALLED NVIDIA Windows driver inside the private sealed image (the owner's own design intent of 2026-11-13: "the NVIDIA driver ... baked in"); this image does the same and nothing else of NVIDIA's is kept: the installer sits only on the build disc (`media/tools.iso` on the pc, a staging file to delete after the backup), the extracted driver tree was deleted from the guest (`C:\kf` absent) and from the host. Read literally the sentence conflicts with the private Windows images; read in its paragraph it is about the Linux apps stage and public artifacts. **This is a question for the owner (section 9).**

## 9. Open items and owner questions

1. **Back up the sealed image** to private storage from the pc (path in 8.1). Until then the pc is the only copy; the working disk `/workspace/pc1/winvm/win11_base/image.qcow2` (20 GB) and `media/` (1.7 GB, holds the NVIDIA installer on the tools disc) can be deleted after the backup.
2. **Ruling scope:** does "nothing from NVIDIA stored in our images" apply to the private Windows images (which hold the installed driver)? If yes, the baked driver of A, B and this image conflicts and the driver would have to be installed at first run, which needs network that these images never have; if no, `docs/DOWNLOADS.md` should say "public artifacts".
3. **`win_vm.sh run` defaults:** rebase #431 to the two properties that are still needed (`guest-driver`, per-VM `gpu-uuid`) and drop the env-file part; or close it and make `win_vm.sh` pass `guest-driver=580.65.06` for Windows by a smaller change. A hardware run of a plain `win_vm.sh run` after the fix is still owed.
4. **`nvidia-smi` telemetry under kf3:** temperature, fan, power, clocks and P-state print `[N/A]` or 0 where the host has numbers (utilisation is #459's). Not a blocker for the base; apps that read those fields would see zeros.
5. **Windows activation:** the image is unactivated, like A and B, and no key was used or invented; if an application layer needs an activated Windows, that is an owner decision (key or evaluation) before the layer is installed.
6. `state.json` has `software: []` and `offline_checks: {}` (as in image B); the software list of the base (NVIDIA 580.88, OpenSSH 10.0.0.0, virtio-win 0.1.302) is in this document, not in the state.

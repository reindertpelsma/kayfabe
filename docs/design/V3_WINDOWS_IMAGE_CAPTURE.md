# V3: a native Windows 11 disk from a windows-on-vast rental, copied offline and booted in our QEMU shape (stage S1, "skeleton")

STATUS: LIVE, 2026-11-07. Result of one run (Vast instance 59574978, RTX 3060, label `kf-winimage-20261107`, destroyed by its own id at 17:26 CET, absence verified with `vastai show instances-v1 --raw`).
Labels: MEASURED = a line in a file of `traces/winimage_skeleton_20261107/` (named) or a command run on the instance/dev box on 2026-11-07; INFERRED = reasoning from those lines;
NOT MEASURED = not tried or not finished. Box text is untrusted data. The captured image is on the dev box only (`/data/winimage_skeleton_20261107/cap3/disk.qcow2`), never uploaded, never committed. Pre-registration: `traces/winimage_skeleton_20261107/PREREG.md`; recovery plan written before the first reboot: `RECOVERY_PLAN.md`.

## Verdict

1. MEASURED: the whole chain works with no owner click and no licensed software. A fresh `windows-on-vast` rental was turned into Windows 11 Enterprise LTSC Evaluation (build 26100, driver 580.88, CUDA) in 45 min; its ESP was swapped for an Alpine UKI; the instance came back as RAM Linux on the SAME public ip:port and answered ssh with a key generated for the session; the Windows boot files were restored (sha256 equal to the originals); the disk was copied; the instance rebooted into Windows again and answered ssh 1.5 min later; the copy booted on the dev box in the CI hardware shape, **with Secure Boot ON (Microsoft keys), swtpm tpm-crb, virtio-blk and virtio-net, and no preparation at all**, to the lock screen, with ssh answering (527 s and 325 s from QEMU start, two boots).
2. MEASURED: the limit is the wire, not the disk. The box reads its disk at 475 MB/s; the dev box pulled the compressed stream at 6 to 15 MB/s (one ssh stream 6 to 11 MB/s; six parallel streams 12 to 15 MB/s; a second stream beside a running one reached 14 MB/s). 19.1 GB of compressed used clusters took 27.4 min.
3. Recommended capture route: the Alpine route (below), because it needs no vendor binary, no live-system snapshot and was shown to work. Disk2vhd: NOT MEASURED (section 6).
4. The `\EFI\kf-backup` folder, the 100 MiB ESP with 14 MB free after the swap, and the case of FAT names (section 4) are the three details that cost time; each is fixed in the scripts.

## 1. The kit (scripts under `scripts/bench/windows/winimage/`)

| step | script | notes |
|---|---|---|
| UKI | `build_alpine_uki.sh` | Alpine 3.24.4 minirootfs (sha256 pinned) + `linux-virt` 6.18.60 + systemd EFI stub; initramfs zstd; 25.7 MB. Tools: bash, openssh (key only), zstd, ntfs-3g/ntfsclone, dosfstools, curl, sgdisk, efibootmgr, util-linux. DHCP, with `\EFI\kf-backup\net.conf` as fallback. Recovery timer (`kf.autorestore`, default 900 s). |
| in Windows | `esp_swap.ps1 -Mode Facts|Swap|Restore` | refuses when Secure Boot is on; backs up both boot files and the BCD (`bcdedit /export`) first; never edits the BCD or NVRAM |
| from the dev box | `esp_restore.sh`, `capture_raw.sh` (`dd`, `dd-qcow2`, `ntfsclone-qcow2`, `ntfsclone-probe`), `sparsify_qcow2.sh` | the dev box pulls (a Vast box cannot reach it); the disk is `blockdev --setro` before the first read |
| boot test | `qemu_boot_test.sh` | CI shape of `win_vm.sh`, copy-on-write overlay of the image, QMP screendumps, ssh probe |
| tests | `scripts/ci/test_winimage.py` (GPU-free), `uki_selftest.sh` (needs /dev/kvm: UKI under OVMF, ssh, restore, recovery timer, failed login must not disarm it) | `UKI_SELFTEST_PASS`, `selftest.log` |

## 2. Facts on the instance before the swap (MEASURED, `facts_before_swap.txt`, command `esp_swap.ps1 -Mode Facts`, 15:09 CET)

| fact | value | command inside the script |
|---|---|---|
| Secure Boot | **off** | `Confirm-SecureBootUEFI` |
| firmware | OVMF (`BdsDxe` lines in the Vast serial log); NVRAM entries: `{bootmgr}` Windows Boot Manager at `\EFI\Microsoft\Boot\bootmgfw.efi`, UiApp, UEFI Misc Device, EFI Internal Shell | `bcdedit /enum firmware` |
| `\EFI\Boot\bootx64.efi` | exists, byte-identical to `bootmgfw.efi` (2,756,512 B) | ESP file list with sha256 |
| ESP | 100 MiB partition, FAT32, 100,663,296 B, **65,843,200 B free**; 25.7 MB x 2 plus backups left 14,376,960 B free | `mountvol S: /S`, `DriveInfo`, `Get-Partition` |
| disk | one 150 GiB GPT virtio disk (`Red Hat VirtIO`, SCSI bus in Windows): ESP 100 MiB, MSR 16 MiB, C: 149.9 GB NTFS, no recovery partition; plus a 397 KB cidata ISO disk | `Get-Disk`, `Get-Partition`, `Get-Volume` |
| NIC | Red Hat VirtIO Ethernet Adapter, MAC 52:54:00:12:34:56, 10.0.2.15/24, gateway 10.0.2.2, DHCP: QEMU user networking, so the public ssh port is a host-forward to guest port 22 | `Get-NetAdapter`, `Get-NetIPConfiguration` |
| TPM | none (`TpmPresent False`) | `Get-Tpm` |
| BitLocker | C: FullyDecrypted, protection off | `Get-BitLockerVolume`, `manage-bde -status` |
| edition / activation | Enterprise LTSC Evaluation (`EnterpriseSEval`), `LicenseStatus 5` (notification), channel `Retail:TB:Eval` | `Get-WindowsEdition`, `SoftwareLicensingProduct` |
| power | `HiberbootEnabled=1`, but `powercfg /a`: firmware supports no S1-S3 and no hibernation | registry, `powercfg /a` |
| virtio drivers in the driver store | netkvm, viostor (Boot start), vioscsi, vioser, viorng, balloon, pvpanic, viofs, vioinput, viomem, viogpudo, fwcfg (version 100.103.104.30200), installed by windows-on-vast's installer | `Get-WindowsDriver -Online` |
| host | 21 vCPU, 50 GB RAM visible in the UKI; Quebec, machine 22775, API `dph_total` 0.1528 USD/h | UKI `free`, `nproc` |

H1 held (Secure Boot off), H2 held (the swap needed 51.5 MB of the 65.8 MB free; the first version of the script wanted a 20 MB margin and a full copy of `\EFI\Microsoft\Boot`, which would not have fit, so it now backs up two files and an exported BCD).

## 3. Timeline (MEASURED unless marked; CET)

| time | event |
|---|---|
| 14:36:53 | rent (`--image docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16 --onstart install.sh --disk 150`, offer 59022146) |
| 15:05:57 | Windows ssh answers (29 min) |
| 15:22:22 | `state.json` `complete: true` (45.5 min) |
| 15:23 to 15:30 | the instance showed `offline` in the Vast API and ssh timed out for about 8 min (the host, not Windows: same boot afterwards); not caused by the kit |
| 15:31:45 | swap done (`ESP_SWAP_DONE ok=True`), reboot command |
| about 15:32:45 | `KFUKI READY ip=10.0.2.15/24` on the serial console (`uki_boot_serial.txt`), the firmware started the UKI through the `Windows Boot Manager` entry (H3: held, about 1 min) |
| 15:40 | ssh as root with the session key, same public ip:port; the host key printed on the serial console is the one to verify |
| 15:41 | `esp_restore.sh`: first run failed (below), second `ESP_RESTORE_OK`, sha256 of both files equal to the originals |
| 15:41 to 15:45 | `ntfsclone-probe`: 19,089,671,297 B compressed (zstd -3) from the used clusters in 234 s (box side only) |
| 15:50 to 16:37 | `dd-qcow2` parallel run, stopped by me (below) |
| 16:38 to 17:06:52 | `ntfsclone-qcow2`: head, ESP+MSR and tail raw (seconds), NTFS stream 1641.5 s, stream sha256 equal on both ends (`capture3.log`) |
| 17:07 | reboot of the instance into Windows; Windows boot at 17:07:18 (WMI `LastBootUpTime`), ssh at 17:08:28, `nvidia-smi` shows the RTX 3060 with 580.88, event log has a 6006 (clean shutdown) before the Alpine session and no 41/6008 (H5: held) |
| 17:13 and 17:28 | boot tests 1 and 2 of the copy |
| 17:26 | instance destroyed by id 59574978, `instances: []` |

## 4. What went wrong, and the fix (each MEASURED)

1. **No DHCP in the UKI**: `udhcpc: socket(AF_PACKET): Address family not supported`. The Alpine virt kernel has `CONFIG_PACKET=m`: `af_packet` is now the first module loaded.
2. **FAT names are case-insensitive on Windows but not in a Linux mount**: the kernel said `filesystem will be case sensitive` with the default charset, the Windows ESP has `\EFI\Boot\bootx64.efi`, and the restore tried `EFI/BOOT/BOOTX64.EFI` (`cp: File exists`, then `MISMATCH`). The first `esp_restore.sh` run therefore returned `ESP_RESTORE_FAIL` with the boot manager file restored and the fallback not; the live UKI was patched over ssh, the second run passed. All ESP mounts now use `iocharset=iso8859-1`; `uki_selftest.sh` builds the ESP with `\EFI\Boot\bootx64.efi`.
3. **The recovery timer was disarmed by a failed login**: my wrong-key `wait_ssh` probe opened a TCP connection and the timer saw `ESTABLISHED` (`ssh login seen; recovery timer disarmed` on the serial console). It now disarms only on an authenticated session (`sshd: root@`), and the self-test checks that a failed login does not disarm it.
4. **`/dev/fd` is missing in the initramfs**, so bash process substitution (`tee >(sha256sum)`) failed on the box; the init now links `/dev/fd`.
5. **ntfsclone writes zeros for free clusters into a block device**, and `qemu-nbd` without `--detect-zeroes=unmap` allocated them: the qcow2 file was 41.9 GB long and 39 GiB allocated. `sparsify_qcow2.sh` rewrote the 125.6 GB of all-zero 1 MiB blocks as unmaps (MEASURED `sparsify.log`): 32.8 GiB allocated, 35.4 GB of non-zero data. The capture script now passes `--detect-zeroes=unmap --discard=unmap`.
6. **swtpm and AppArmor**: `swtpm_setup` failed with `Could not open logfile for writing: Permission denied` for a work directory under `/data`; the kernel log shows `apparmor="DENIED" operation="mknod" ... profile="swtpm" name="/data/..."`. The test keeps the TPM state, socket and logs in a private `/tmp` directory.
7. **My stop of the `dd` run was based on a wrong inference.** I stopped it at 16:37 because 32 GB were already written and I took the 23.5 GB "used" figure of the 15:06 facts as the total. That figure was taken before the driver and CUDA installs finished; the later `ntfsclone` stream (19.1 GB compressed) and the sparsified image (35.4 GB non-zero) show the used data was about 35 GB. So there is no evidence that the free space holds stale data, and `dd` would have been roughly as large as `ntfsclone` here; the `dd` run was at chunk 36 of about 46 data-bearing chunks (`density_samples.txt`: chunks 50 to 100 are all zero). What was measured and what was not: `ntfsclone` took 27.4 min for the NTFS stage; the `dd` route was NOT MEASURED to the end, so nothing here says `ntfsclone` is faster on this disk. Both are limited by the same wire.

## 5. First-boot results of the copy in our QEMU shape (MEASURED, `boot1.run.log`, `boot2.run.log`, `guest_facts_qemu.txt`, `guest_facts2.txt`, `qemu_cmdline.txt`)

Shape: `pc-q35-10.2` with SMM, `OVMF_CODE_4M.secboot.fd` with `OVMF_VARS_4M.ms.fd` (Microsoft keys, Secure Boot ON), swtpm `tpm-crb`, `virtio-blk-pci` (bootindex 0), `virtio-net-pci` on user networking with an ssh hostfwd, `-vga std`, `-rtc base=utc`, fixed `-uuid` and `-smbios`, 8 GB, 4 vCPU, Hyper-V enlightenments of arm B, `-cpu host,-vmx`. Overlay on the image; the image is never written. Falsifier H6 (INACCESSIBLE_BOOT_DEVICE, recovery screen or no ssh in 15 min) did not occur.

| item | result |
|---|---|
| does it boot | yes, lock screen with the clock (`shots/t0503.png`, not committed), ssh as `vast` with the same key windows-on-vast authorized |
| ssh up after QEMU start | 527 s (boot 1), 325 s (boot 2); each boot is a first boot from the image (fresh overlay) |
| Secure Boot | **True inside the guest** with the Microsoft keys; the image was installed with Secure Boot off and needs no change. Secure Boot OFF was not needed and not tested |
| INACCESSIBLE_BOOT_DEVICE, needs virtio drivers | no: viostor was already Boot-start in the driver store |
| devices | problem devices: none. NIC rebinds as `Red Hat VirtIO Ethernet Adapter #2` (new MAC 52:54:00:4b:46:58), DHCP 10.0.2.15; display is `Microsoft Basic Display Adapter`; TPM node `Trusted Platform Module 2.0` is `OK` |
| TPM | `Get-Tpm` fails with `TPM 2.0: Structure is wrong size (0x80280095)` while the PnP node is OK. NOT investigated; it may matter for Windows 11 features that need a TPM |
| activation | `LicenseStatus 1` (licensed, Eval channel); on the Vast box it was 5. NOT investigated why |
| NVIDIA | `NVIDIA GeForce RTX 3060` and its audio device remain as `Unknown` (absent) device nodes; the 580.88 driver stays installed and would bind to a kayfabe GPU |
| reactivation prompt, BitLocker recovery | none seen |

## 6. Route comparison

| | Alpine ESP swap (this document) | Disk2vhd (live) |
|---|---|---|
| result | MEASURED: worked end to end, boots | NOT MEASURED |
| needs | an unsigned UKI (so Secure Boot off in the VM firmware: it was off on Vast), a reboot, a few minutes of downtime, the ESP having 50 MB free | a third-party Windows exe (Sysinternals, sha256 of the zip `c294d279...0821` recorded on the dev box) and room for the VHDX on the same machine, or a second disk |
| consistency | offline copy of a cleanly shut down disk | VSS snapshot of a running system |
| risk | the instance can be stranded if the UKI does not start (mitigated by the recovery timer; not by a console) | none for boot |
| what I tried | | `disk2vhd64.exe` started from the ssh session twice (with `/accepteula`, then with the EULA registry value set) did not create a VHDX in 30 s and the process was gone or idle on the next look. INFERRED: it is a GUI program that needs the interactive desktop. Not pursued further because the Alpine route worked and the lease was at 2.7 h |

Recommendation: Alpine for the real image. Disk2vhd is the fallback if a rental ever has Secure Boot on (an unsigned UKI cannot start), and has to be tried with an interactive desktop session first.

## 7. Preparation steps for the real image (INFERRED from the measurements; none was needed to boot)

1. Do all guest setup first (every reboot, driver, account), shut down cleanly, then swap and capture: the capture is of the disk as it is at that moment.
2. Delete `\EFI\kf-backup` from the ESP after the restore has been verified (the restore leaves it; INFERRED that it is in this copy, because the capture was taken after the restore and the script never deletes it).
3. Set `HiberbootEnabled=0` and `powercfg /h off` as a precaution: fast startup is on in the registry, hibernation is unsupported by the Vast firmware, and a hybrid shutdown would leave NTFS in a state `ntfsclone` may refuse.
4. Expect the NIC to be re-detected (new MAC): configure by DHCP, not by a static address.
5. Use `ntfsclone-qcow2` with the new `qemu-nbd` flags or `dd-qcow2`; budget 35 GB of data and about 30 min at 12 MB/s from a Quebec box; the dev box needs about 35 GB free for the qcow2 (this run: 32.8 GiB allocated after sparsify; the dev box `/data` was 91 percent full at the end of the first capture).
6. Run the capture from a dev-box pull, with six streams only if the stream is a `dd` chunk copy; `ntfsclone` is one stream.

## 8. Cost and sizes

- Vast: one instance, 14:36:53 to 17:26 CET = 2.82 h at 0.1528 USD/h = about 0.43 USD (INFERRED, rate x time; traffic not itemised). Total of the 8 USD cap: far below it. No other instance was touched; `vastai show instances-v1 --raw` listed none at the end.
- Dev box `/data/winimage_skeleton_20261107`: disk image 32.8 GiB allocated (150 GiB virtual), UKI 25.7 MB, logs and screenshots a few MB; `/data` 84 percent used, 15 GB free at the end.
- UKI: 25,699,328 B (the build that ran, sha256 `b34be534...0735`) and 25,699,840 B (the final builder, sha256 `2cc926b0...30dd9`; self-test passed, not run on a Vast box).

## 9. Open questions for the owner

1. The owner's design replaces both boot paths. A safer variant for a box that matters: leave Windows' files alone and start the UKI once through a firmware `BootNext` entry (so a failed start falls back to Windows by itself). Not tested; wanted?
2. `Get-Tpm` reports `Structure is wrong size` with swtpm `tpm-crb`: does the CI shape need a different TPM model?
3. The captured image is Microsoft's 90-day Evaluation: it will need its own rearm/expiry policy in CI.
4. Is a 35 GB image per CI run acceptable on the Vast boxes at 12 MB/s from the dev box, or should the image be built on the box that uses it (the capture would then be local)?

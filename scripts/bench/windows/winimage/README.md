# winimage: take a native Windows 11 disk from a windows-on-vast rental and boot it in our QEMU shape

STATUS: LIVE, 2026-11-07 (skeleton, stage S1). The kit is tested GPU-free (`scripts/ci/test_winimage.py`, `uki_selftest.sh`);
what it did on a real rental, and what did not work, is in `docs/design/V3_WINDOWS_IMAGE_CAPTURE.md` and
`traces/winimage_skeleton_20261107/`. Nothing here is a product feature and nothing here may be run against a machine that
holds data you care about: the swap replaces the boot files of the Windows it runs in.

Route (the owner's design, 2026-11-07): a rented `windows-on-vast` Windows 11 (no owner clicks, no licensed software) is
copied offline, from a RAM Linux that replaced the Windows boot files for one boot, then converted and boot-tested on the dev box.

| step | where | script |
|---|---|---|
| 1. build one UEFI executable: Alpine kernel + initramfs (DHCP, sshd on 22 with one key, zstd, ntfs-3g, dosfstools, curl) | dev box (root) | `build_alpine_uki.sh` |
| 2. facts, then back up the Windows boot files on the ESP and replace BOTH boot paths with the UKI | in Windows | `esp_swap.ps1 -Mode Facts`, `-Mode Swap` |
| 3. reboot; the box answers ssh on the same public ip:port, as root, with the session key | dev box | `ssh -i KEY -p PORT root@IP` |
| 4. put the Windows boot files back (`kf-restore-esp`), then copy the raw disk (read-only, `dd | zstd`, pulled by the dev box) | dev box -> UKI | `esp_restore.sh`, `capture_raw.sh` |
| 5. reboot the instance into Windows and check that it boots | dev box | `esp_restore.sh ... --reboot` |
| 6. the qcow2 is written directly by `capture_raw.sh dd-qcow2|ntfsclone-qcow2` (through qemu-nbd, so no raw copy and no `.zst` file next to it; `sparsify_qcow2.sh` frees all-zero clusters in place), then boot under plain QEMU/KVM in the CI shape, screenshots by QMP | dev box | `qemu_boot_test.sh` |

Why the dev box pulls: a Vast box cannot reach the dev box (NAT), the dev box can reach the box's public port.

## Safety properties (each has a falsifier test in `scripts/ci/test_winimage.py`)

- The UKI image holds only the PUBLIC key; the builder refuses a file that holds a private key. No password login. Host keys are
  generated at every boot. The private key stays on the dev box (`/root/night/winimage_private`, mode 0700/0600) and is deleted at the end.
- Recovery without a console: `kf.autorestore=N` (default 900 s) in the UKI command line. When nobody logged in over ssh within N seconds,
  the UKI restores the Windows boot files from `\EFI\kf-backup` and reboots. A failed network can therefore not strand the instance.
- `esp_swap.ps1` refuses when Secure Boot is on, when the UKI hash is not the expected one, when there is no Windows boot manager file, or
  when the ESP lacks room; it never overwrites an existing backup with the UKI; it never edits the BCD store or the NVRAM.
- `capture_raw.sh` sets the disk read-only (`blockdev --setro`) before the first read and never writes to a block device.

## Tests

```
python3 -I scripts/ci/test_winimage.py                       # GPU-free, no KVM, no network
UKI=... KEY=... bash scripts/bench/windows/winimage/uki_selftest.sh [DIR]   # needs /dev/kvm: boots the UKI under OVMF, ssh, restore, recovery timer
```

<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# gop_standin_20261003 — kf3's GOP option ROM on a stand-in device, no GPU

⊘ **2026-10-03 (late) — read this run as history.** Its revisions (`3dd574e5`, `2c6178fe`, `da5cc07f`) are
on branch `v3-gop-rom`, which carries the committed `kf-gop.efi` that `OWNER_RULINGS.md` §K withdrew and
is not merged; branch `v3-gop` squashes that work without the binary. The blob it packed (`f11ab0b9…`) no
longer exists in the tree, and three of its Secure Boot arms were renamed `*_observe` and no longer
PASS. The run at `v3-gop`'s head, with a ROM-verifying OVMF, is `../gop_standin_20261003_v3gop/`.

**STATUS: MEASURED, 2026-10-03 — 11/11 arms PASS at `3dd574e5` (branch `v3-gop-rom`, clean worktree).**
Local dev host, no GPU and no kf3: QEMU 10.2.1 (Ubuntu `1:10.2.1+ds-1ubuntu3.2`), KVM, 512 MiB, 1 vCPU,
q35. Firmware: Ubuntu `ovmf 2025.11-3ubuntu7` (`OVMF_CODE_4M.fd`, `.secboot.fd`, VARS plain / `ms` /
`snakeoil`) and QEMU's own `edk2-x86_64-code.fd` from QEMU 10.2.4's `pc-bios` (the same `.bz2` as QEMU
11.1.1's: md5 `ef441937…` for both). Guest kernel for the Linux arms: the host's `7.0.0-34-generic`
with a busybox initramfs.

Command (`docs/design/V3_DISPLAY.md` §4.11.8):

```sh
rustup target add x86_64-unknown-uefi --toolchain 1.99.0
bash scripts/display/gop_standin.sh          # all arms; --keep keeps the scratch dir
```

The stand-in device is QEMU's `ati-vga` (1002:5046, class 0x0300), which OVMF has no built-in driver
for — the position a real NVIDIA GPU is in. The ROM is the release `kf-gop.efi` (9 216 bytes,
sha256 `f11ab0b9…` — the committed blob `firmware/kf-gop/kf-gop.efi`) packed by `kf-oprom` for 1002:5046, BAR 0, 1152x648, pitch 4608, G = 0x2E0000.

| file | what |
|---|---|
| `standin.log` | the script's whole output: build identity, the PE checks, the packed ROM, one `GOP_STANDIN arm=…` verdict line per arm, the summary |
| `arms_debugcon.log` | per firmware arm: kf-gop's own debug lines (debugcon builds only) and the test app's 21 checks; the Secure Boot arm's boot-manager lines from the serial console |
| `linux.serial.log`, `linux_two_vga.serial.log` | what the guest kernel reported: vgaarb, simpledrm, `/proc/iomem`, each display device's `boot_vga` and command register, sysfb's parent, fb0 |
| `qemu_edk2_excerpt.log` | QEMU's DEBUG edk2 on the option ROM's deferral and on GraphicsConsole starting on kf-gop's mode |

Headlines (the per-arm table and what follows from it are in `V3_DISPLAY.md` §4.11.8):
- every GOP check passes and the framebuffer read back through QMP `pmemsave` equals the drawn
  pattern in 648/648 lines, on both OVMF builds;
- Linux's EFI framebuffer is the device's BAR0 at offset 0, `pitch·H` = 2 985 984 bytes long, owned by
  simpledrm, with the PCI device as sysfb's parent and `boot_vga` = 1, and a pattern Linux writes
  through `/dev/fb0` is in the BAR byte-exact (648/648 lines at pitch 4608); with a second,
  driverless VGA device in a lower slot, vgaarb first picks that one and then `overriding
  previous` moves to the device holding the firmware framebuffer;
- OVMF runs the ROM driver whether or not it is signed, also with Secure Boot enforcing (the
  unsigned boot application is `Access Denied` in the same boot).

The same script runs in CI (job `firmware`). Run 37127211692 at `2c6178fe` (2026-10-03, GitHub
`ubuntu-latest`, KVM, QEMU 8.2.2, Ubuntu `ovmf 2024.02-2ubuntu0.9`, kernel `6.17.0-1022-azure`):
10 arms PASS, `gop_qemu_edk2` SKIP (no QEMU edk2 build installed there) — a third OVMF build. That
revision's Linux arm still had the weaker last check (non-zero BAR bytes), replaced at `da5cc07f`;
run 37127710871 at `da5cc07f`: the same 10 PASS and 1 SKIP, the `/dev/fb0` pattern 648/648 lines
byte-exact on `6.17.0-1022-azure`.

Local paths in `standin.log` are replaced by `<scratch>` and `<target>`. No VM image is kept.

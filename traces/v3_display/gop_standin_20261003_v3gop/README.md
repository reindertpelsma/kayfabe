<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# gop_standin_20261003_v3gop — the boot-display GOP stand-in at `v3-gop`, with a ROM-verifying OVMF

**STATUS: MEASURED, 2026-10-03 (late) — 14 arms at `adbe6fcd` (branch `v3-gop`, clean worktree):
11 PASS, 3 OBSERVED, 0 SKIP, 0 FAIL.** Local dev host, no GPU and no kf3: QEMU 10.2.1 (Ubuntu
`1:10.2.1+ds-1ubuntu3.2`), KVM, 512 MiB, 1 vCPU, q35; Ubuntu `ovmf 2025.11-3ubuntu7`; QEMU 10.2.4's
`edk2-x86_64-code.fd`; guest kernel `7.0.0-34-generic` with a busybox initramfs. The driver is
`firmware/kf-gop` built by the script itself (release 9 216 bytes, sha256
`e2616ad8d4cb09da88399218144b6bdea9d2cb3148b701964f926604b7d6dff6`) — the same bytes
`crates/kf-gop-image` embeds (CI run 37136448638's `firmware` job: *"reproduced across target
directories"* with that sha256; nothing compiled is committed, `docs/OWNER_RULINGS.md` §K).

Command:

```sh
JOBS=3 bash scripts/display/build_ovmf_deny.sh /workspace/bench/qemu-10.2.4/roms/edk2 <out>
OVMF_DENY_CODE=<out>/OVMF_CODE_4M.deny.fd OVMF_DENY_VARS=/usr/share/OVMF/OVMF_VARS_4M.snakeoil.fd \
  bash scripts/display/gop_standin.sh --keep
```

The deny-policy firmware: edk2-stable202408 (QEMU 10.2.4's `roms/edk2`), `OvmfPkgX64.dsc` RELEASE GCC5,
Secure Boot + SMM + 4 MiB flash, `PcdOptionRomImageVerificationPolicy` 0x04 instead of stock 0x00. This
run's build: sha256 `827402e532f2254f74b086358463454a71708e3b718d519abd6c1fd2f5981b90` (EDK2 builds embed
timestamps, so a rebuild differs in bytes). It is not committed; the script rebuilds it in ~3 minutes.

| file | what |
|---|---|
| `standin.log` | the script's whole output: build identity, the PE checks, one `GOP_STANDIN arm=…` line per arm, the summary |
| `secure_boot_arms_debugcon.log` | the three `sb_deny_*` arms, `sb_snakeoil_signed` and `sb_snakeoil_tailpad_observe`: kf-gop's debug lines (present only when the ROM driver ran), the test app's checks, the boot manager's serial lines |

Headlines:
- ★ **Test F1 (`docs/design/V3_DISPLAY.md` §4.11.9) passes.** On firmware that verifies option ROMs, the
  snakeoil-signed ROM in kf-oprom's end-aligned layout is verified and runs (`kf-gop: loaded`,
  `started`, the test app's 21/21), while the unsigned ROM and the **same signed PE with padding after
  it** do not run at all (no `kf-gop:` line; the signed test app finds no GOP). That is the premise of
  `crates/kf-oprom/src/pack.rs`'s layout — bytes after a signed PE break its verification — observed,
  where until now it rested on reading `DxeImageVerificationLib.c`.
- On stock OVMF (policy 0x00) the same three ROMs all run (`*_observe` arms, `verdict=OBSERVED`), as in
  the first run (`../gop_standin_20261003/`).
- Every other arm as in the first run: 21/21 GOP checks and the framebuffer byte-exact (648/648 lines)
  on both OVMF builds — now through the driver's aligned volatile word stores; refusals by name; Linux's
  simpledrm on BOOTFB at BAR0 + 0 with `boot_vga` = 1 and `/dev/fb0` byte-exact; `boot_vga` following the
  firmware framebuffer with two VGA devices; OVMF's built-in driver winning stdvga.

⊘ What this does not establish: anything on a GPU box (kf3, BAR1, RM's console adoption — box tests
B0a–B10), AAVMF/arm64 (test A1), or a firmware with Microsoft's keys and a deny policy (a physical
machine's; this run uses the snakeoil keys).

Local paths in `standin.log` are replaced by `<scratch>`, `<target>` and `<out>`. No VM image or firmware image is kept.

<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# firmware/ — guest-side UEFI code

**STATUS: LIVE, 2026-10-03.** Code here runs **inside the guest's firmware** (OVMF), not in the
VMM. It is built for `x86_64-unknown-uefi`, outside the root cargo workspace
(`Cargo.toml`: `exclude = ["archive", "firmware"]`), and never linked into QEMU or any `kf-*` crate.

## `kf-gop/` — kf3's boot-display GOP

A zero-dependency, `no_std` UEFI **boot-service driver** that kf3 carries in its PCI expansion ROM
(`docs/design/V3_DISPLAY.md` §4.11). It binds only to the controller whose ROM image holds a `KFGP`
descriptor naming that controller's own vendor and device ids, enables memory decode and nothing
else, and publishes one GOP mode whose framebuffer is the descriptor's range of the descriptor's BAR
(kf3: BAR1 offset 0). `Blt` is software, with a RAM shadow for reads. It programs no hardware.

| path | what |
|---|---|
| `src/lib.rs`, `blt.rs`, `probe.rs`, `log.rs` | every decision, safe (`forbid(unsafe_code)`), host-tested |
| `src/main.rs` | the driver binary's root: module list and panic handler |
| `src/efi_unsafe.rs` | the UEFI ABI (tables, protocols, GUIDs) and boot-service wrappers |
| `src/driver_unsafe.rs` | the entry point, driver binding, component name, GOP |
| `src/port_unsafe.rs` | debug output on I/O port 0x402 — compiled into the driver only with `--features debugcon` |
| `src/bin/kf-gop-test/` | a UEFI application that checks the GOP; used by the local stand-in, never shipped |
| `build.rs` | links the driver as PE subsystem 11 (`/subsystem:efi_boot_service_driver`), reproducibly (`/Brepro`, `/DEBUG:NONE`) |
| `kf-gop.efi` | the committed release build: the blob `crates/kf-oprom` embeds and kf3 serves |

The descriptor has one definition, `crates/kf-oprom` (pure, `no_std`), which this crate depends on
by path with default features off; kf3 packs the ROM with the same crate.

⊘ **CORRECTED by the owner the same day (`docs/OWNER_RULINGS.md` §K) — the paragraph below is to be
withdrawn:** *"We aren't going to put compiled stuff in the repo right? … the efi driver is compiled when
building the repo."* build.rs is to compile this crate during the normal cargo build (the nested-cargo
pattern of `crates/kayfabe-isolate-host/build.rs`), with `x86_64-unknown-uefi` in `rust-toolchain.toml`,
and the ROM kept arch-neutral for a later aarch64. ⚠ Not done yet (2026-10-03): the blob below is still
committed and still what kf3 serves (`docs/design/V3_DISPLAY.md` §4.11.6, §4.11.12).

**`kf-gop.efi` beside the source is the shipped driver.** Owner, 2026-10-03: the GOP is one constant
blob inside kayfabe, like the PTX kernels. `crates/kf-oprom` embeds this file (`KF_GOP_EFI`,
`include_bytes!`) and kf3 wraps it at realize with per-device data; nothing is installed. CI rebuilds it
and requires the same bytes — the link is reproducible (`build.rs`: `/Brepro`, `/DEBUG:NONE`), so
nothing is normalised. After any change to this crate or to `crates/kf-oprom`'s descriptor code:

```sh
cargo build --release --target x86_64-unknown-uefi --manifest-path firmware/kf-gop/Cargo.toml
cp firmware/kf-gop/target/x86_64-unknown-uefi/release/kf-gop.efi firmware/kf-gop/kf-gop.efi
```

### Build

```sh
rustup target add x86_64-unknown-uefi --toolchain 1.99.0      # once; rust-toolchain.toml stays as pinned
cargo build --release --target x86_64-unknown-uefi --manifest-path firmware/kf-gop/Cargo.toml
#   → firmware/kf-gop/target/x86_64-unknown-uefi/release/kf-gop.efi        (the ROM driver)
#   → firmware/kf-gop/target/x86_64-unknown-uefi/release/kf-gop-test.efi   (the test app)
cargo test --lib --target x86_64-unknown-linux-gnu --manifest-path firmware/kf-gop/Cargo.toml
cargo run -p kf-oprom -- pe firmware/kf-gop/target/x86_64-unknown-uefi/release/kf-gop.efi   # kf3's acceptance check
```

Add `--features debugcon` (and another `--target-dir`) for a test build that reports on port 0x402.
The CI job `firmware` builds both flavours, compares the release build with the committed
`kf-gop.efi` byte for byte, runs the host tests, Clippy with warnings as errors, rustfmt, the PE check
(and that the release driver contains no port I/O), records the `.efi` size, and runs the local
stand-in.

### Test without a GPU

`scripts/display/gop_standin.sh` packs this driver for QEMU's `ati-vga` and boots it under OVMF:
every GOP call, the framebuffer read back byte-exact, the refusals by name, Linux's view of the
firmware framebuffer, and Secure Boot. Results and what they established: `V3_DISPLAY.md` §4.11.8.

## The unsafe exception, and how it is contained

`unsafe` is otherwise allowed only in `kf-linux-raw`, `kf-qemu` and `kf-cuda` (and the retained
grader's two adapters). Firmware that talks to UEFI cannot avoid raw tables, so `firmware/` is a
**named exception** (owner default, 2026-10-03; open owner question 2 of the GOP design) under the
same rules, enforced by the `stable` CI job:

- `unsafe` only in files named `*_unsafe.rs` (the unsafe-surface gate), and raw pointer types only
  there (the host-pointer gate);
- `*_unsafe.rs` files only under `firmware/kf-gop/src/` (gate B), counted by the ratchet
  (`AUDITED`: `firmware/kf-gop:77`) — a new relaxation changes the number in the same commit;
- a `#[repr(C)]` layout under `firmware/` only in a `*_unsafe.rs` file (the ABI-quarantine gate's
  firmware arm);
- Clippy's `undocumented_unsafe_blocks` is denied: every block names its invariant.

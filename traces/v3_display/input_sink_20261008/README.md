STATUS: RESEARCH — trusted host, 2026-10-08. Hardware proof of KF3 ABI 23 (`OWNER_RULINGS.md` §V;
`docs/design/V3_DISPLAY.md` §8.20): the broker's input and console-cursor policy in `kf-broker`
behind the VMM-neutral `InputSink`/`CursorSink`, kf3.c reduced to the verbs.

Binaries: kf3 `cc43d9aa` (this branch, `build_kf3.sh`, own `CARGO_TARGET_DIR`); control kf3
`4bc62999` (the previous agent's §8.19 build — `crates/kf-broker`, `kf3.c` and `kf-qemu`'s
`broker.rs` are byte-identical between `4bc62999` and this branch's base `cff7823a`, checked with
`git diff --stat`). Broker nvkvm-pv `badf2d7` (`/opt/nvkvm-broker`). RTX 4070, host 595.91.07.

## Falsifiers, stated BEFORE the runs (committed before the first boot)

Run `isc` (control, kf3 `4bc62999`) and `is1` (kf3 `cc43d9aa`), both `input_proof.sh` from this
checkout; `is1` adds `PROOF_REBOOTS=1 PROOF_REATTACH=1`:

- H1 "the refactor changed nothing the guest sees" is FALSIFIED by any of: a PROOF line among
  GRUB_KEY, ABS (each position), REL_X11, ABS_UNDER_GRAB, KEYS that passes in `isc` and fails in
  `is1`; an EVDEV_MOUSE / EVDEV_TABLET / EVDEV_KEYBOARD line of `is1` that differs from `isc`'s
  (same script, same injected sequence) in REL sums, button edges, wheel count or key edges;
  GRAB_ON / GRAB_OFF naming a different device (`#5 QEMU Virtio Mouse (relative)` /
  `#4 QEMU Virtio Tablet (absolute)` expected in both); the broker restart (BROKER2) not
  reconnecting; a reboot with an Xid, `RmInitAdapter failed`, or no desktop.
- The ABS falsifier of §8.17 stands: guest pointer more than 1 px from `x - 1` for x in the
  window's range (QEMU's `x * 32767 / W` scale).

Run `ig1` (`grab_repro.sh`, kf3 `cc43d9aa`, CASES A B C D), compared with `g2` (`4bc62999`, A B)
and `g1` (`2a20e699`, A-D):

- H2 "grab routing is unchanged by the move" is FALSIFIED by: any case whose core or raw sum is not
  (200, 0); any core jump above the per-packet step (10); in case B any button or wheel event on the
  TABLET (g2: `tablet_events=0`, `mouse_btn=2`, `mouse_wheel=2`); in case D an ABS reaching the
  guest during the grab (`tablet_events` > the ungrab re-sync).

## Results

(filled in after the runs; measured lines are quoted from the logs in this directory)

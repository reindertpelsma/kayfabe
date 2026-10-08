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

## Results (measured 2026-10-08, runs isc 16:29-16:32, is1 16:32-16:36, ig1 16:36-16:38; `chain.log`)

Before the chain the GPU was held for ~70 min by two other agents' runs (sub-pixel proof, Windows
boundary runs 61/66) and an idle Windows desktop VM (`kayfabe-windows-desktop`, up since 15:43, no
sampler attached), which this agent powered down by ACPI at 16:25:55 (`system_powerdown` over its QMP
socket; recorded in `chain.log`). Another agent launched Windows boundary run 67 at 16:34:27, i.e.
during is1; is1's graded lines are unaffected (below), and its REATTACH line's broker pids are
`input_proof.sh`'s newest-broker pick, which caught that run's broker — an instrument caveat, not a
kayfabe result (the relay line shows the reattach).

- **H1 not falsified (is1 vs isc).** Every PROOF line of `is1_proof.log` equals `isc_proof.log`'s:
  GRUB_KEY PASS; ABS PASS at all 8 positions (got = x - 1, as §8.17); GRAB_ON `#5 QEMU Virtio Mouse
  (relative)`; REL_X11 PASS (70, 30); ABS_UNDER_GRAB PASS_DROPPED; GRAB_OFF `#4 QEMU Virtio Tablet
  (absolute)`; BROKER2 reconnected; EVDEV_MOUSE `rel 70,30 btn=[BTN_RIGHT=1 BTN_RIGHT=0] wheel=1`;
  EVDEV_TABLET `abs_events=18 btn=[BTN_LEFT=1 BTN_LEFT=0] wheel=3`; KEYS PASS. The guest's evdev
  streams are identical with timestamps removed: keyboard 40, mouse 37, tablet 35 events in both
  (`*_ev_*.log`). 4 pointing-device switch lines in each QEMU log, 0 missing-device warnings.
  is1 alone: REBOOT1 `xid=0 rminit_fail=0 wpr=0 desktop=up`, REATTACH_DISPLAY `stalled=0`.
  Status at the end of is1: `input[keys=36 keys_refused=0 keys_unmapped=0 buttons=4
  buttons_dropped=0 wheel=2 abs=10 rel=16 switches=4]`.
- **H2 not falsified (ig1).** Cases A-D: core (200, 0), raw (200, 0), 0 jumps, max core step 10 in
  each; B and C: `tablet_events=0 mouse_btn=2 mouse_wheel=2` (g2's B line, value for value); D:
  `tablet_events=0` (the ABS was dropped). The relay counted `0 absolute report(s) dropped` in the
  intervals logged (`ig1_relay_grab_lines.txt`).
- **Demo left running** (kf3 `cc43d9aa`, real Wayland broker, `run-20261008-163844`): at 16:39 the
  guest's Cinnamon was up, `nvidia-smi -L` answered, `broker[up=1 sent=178 gpucopy=176 …]`.

# The interactive broker window on the trusted host — input proof, reboots, unload (2026-10-08)

**STATUS: RESEARCH, 2026-10-08.** Host: the trusted host (RTX 4070 AD104, host driver 595.91.07 open,
Ubuntu 26.04, kernel 7.0, live GNOME **Wayland** session of user `ubuntu`, uid 1000). kf3 binary
`kf3-bins/0e64a960` (integration/master-candidate-20261008) in every run. Broker: nvkvm-pv
`broker-cursor-gpucopy` at **`badf2d707da4`**, exported with `git archive` from `/root/nvkvm-pv` and built
into `/opt/nvkvm-broker` (backends: wayland, x11 + xi2 raw motion, test). Guest: an overlay of
`/workspace/bench/guest.qcow2` (Ubuntu noble 6.8.0-142, open 580.159.04) with Cinnamon/lightdm/Xorg,
evtest, xdotool, xinput installed by `interactive.sh prep` (689 packages). Harness:
`scripts/bench/display/interactive.sh` and `input_proof.sh` at the checkout revisions named in each log's
first line (`a372a279` p1, `b435bbd9` p4 and wl1). V3_DISPLAY.md §8.17 is the design-side record.

| file | what |
|---|---|
| `p4_proof.log` | run p4 (`KF3_RPC_TRACE=1 PROOF_UNLOAD=1 PROOF_REBOOTS=2 PROOF_REATTACH=1 input_proof.sh p4`): every PROOF_* grade |
| `p4_ev_{Keyboard,Tablet,Mouse}.log` | the guest's evtest on the three virtio input devices during p4 |
| `p4_grub_menu.png`, `p4_grub_editor.png` | kf3 console readback (QEMU `screendump kf0`, the frame the broker is sent) — grub's menu, then its editor opened by the broker's `e` |
| `p4_desktop.png`, `p4_reboot2_after.png` | the NVIDIA X desktop (Cinnamon in fallback mode) after the first boot and after the second reboot |
| `p4_after_unload_15s.png` | 15 s after `modprobe -r nvidia_drm nvidia_modeset nvidia`: all pixels 0 |
| `p4_nv0073_rpc_census.txt` | every NV0073 control the guest sent kf3's GSP in p4 (3 boots), from `KF3_RPC_TRACE=1` |
| `p4_broker.log`, `p4_relay_lines.txt` | the test-backend broker's log, kf3's relay lines |
| `ab1..ab4.log` | the A/B runs on the first-copy stall (configs in each line) |
| `p1_proof.log`, `p3_proof.log` | the two runs the stall killed |
| `wl1_*` | the owner-demo launch (`interactive.sh`, the real Wayland broker on GNOME), started 11:42 and left running for the owner |

The PNGs are the kf3 console's framebuffer readback, not a photo of the host window. No host desktop
screenshot is committed (it is the owner's live session).

**STATUS: LIVE, 2026-10-14.** Evidence of the Linux plane oracle (`docs/design/V3_DISPLAY_PLANES_ORACLE.md`).

| dir | what | kf3 / driver |
|---|---|---|
| `run1-bf510bdf/` | kayfabe before: the enable + both-planes deadlock (12 STALL reports, the core parked; `kf3-display-lines.txt.gz` holds the engine rings) | kf3 `bf510bdf`, guest open 580.159.04 |
| `run2-a34d3003/` | the vblank-parked join removed: flips complete, disables raise nothing (45 guest Flip event timeouts) | `a34d3003` |
| `hw-ad104-vast/` | the bare-metal reference: RTX 4070 SUPER (AD104) in a vast KVM VM by VFIO, open 580.159.04, HDMI forced on with the kayfabe EDID (see `NOTE.txt`) | real hardware |
| `run3-113a7535/` | + the flip-away AWAKEN and the semi-planar caps: every series incl. NV12/P010 completes, 0 guest timeouts | `113a7535` |
| `run4-41acc6ac/` | + the console YUV kernel: NV12/P010/YUYV pixel-exact on the console | `41acc6ac` |

Each run directory: `summary.log` (the host runner's `PLANES_*` lines, incl. host Xid delta and the owner VM's
liveness), `all.log`/`list.log` (kfplanes), `guest-dmesg.log`, `show-*.log`/`cmp-*.log` (console composition),
`kf3-display-lines.txt.gz` (kf3's display log lines without the per-second status), `hypothesis-*.txt` (written
before the run). Regenerate the comparison table: `python3 scripts/bench/display/planes_table.py <this dir>`.
Host paths (not in git): `/var/lib/kf-windows-20261005/claude-planes-<run>/` (incl. qemu.log, screendumps).

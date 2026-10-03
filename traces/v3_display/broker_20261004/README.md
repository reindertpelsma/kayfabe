# The broker lane on hardware after nvkvm-pv `badf2d7` — console cursor, XOR, alpha, refusals (2026-10-04)

Box: vdisp = vast 54032077, RTX 3060 (GA106), host driver 580.159.04, Ubuntu 22.04.5, KDE Plasma on
Xorg (NVIDIA DDX) via sddm; guest Ubuntu noble with 580.159.04, Cinnamon on Xorg. Runs at
2026-10-03T23:15Z–23:31Z (UTC). Harness `scripts/bench/display/broker_lane.sh` (`prep`, `run`) and
`broker_hook.sh`, both from the box's GitHub checkout; the box state before and after is in
`box_state.txt`, prep and the three builds in `prep_and_build.txt`.

**Revisions.** Broker: nvkvm-pv `broker-cursor-gpucopy` at **`badf2d707da4`** in every run (built on the
box from GitHub by `prep`, `/opt/nvkvm-broker/nvkvm-display-broker` sha256 `6d07e97718afe222`). kf3,
the binary's path stamp (`run_<tag>_rev.txt`):

| run | kf3 | what the run is |
|---|---|---|
| `brkF1` | `49f1e1df` | the lane with `BRK_VNC=1 BRK_DRI3=1 BRK_RESILIENCE=1` (the DRI3 client aborted on its own bug, fixed in `3aeac81b`) |
| `brkF2` | `3aeac81b` | `BRK_VNC=1 BRK_DRI3=1 BRK_HOLD=1200`: the DRI3 client, the cursor steps, then the scaled-cursor checks by hand (`brkF2/scaled/`) |
| `brkF3` | `cc19632e` | `BRK_DRI3=1 BRK_CURSOR=0`: the DRI3 client with three more variants, then the VMM on the same broker |

All three binaries carry the implementer's code at **`34696441`**: the three commits after it change
`scripts/bench/display/` only, and the Rust archive is the same file in all three builds (one mtime,
`prep_and_build.txt`). Every run: guest dmesg persisted (`run_<tag>_dmesg.log`: 25 NVRM lines;
`_dmesg_after.log`: 64, 61 and 56), host dmesg 0 Xid (none in the whole host log, `box_state.txt`), the
session `LockedHint=no` at the start.

## Grades

| item | result |
|---|---|
| **Rung 0 carries frames and they are released** | PASS. `brkF2`: `sent=7263 gpucopy=7262 releases=7262 reclaims=1`; `brkF3`: `sent=1468 gpucopy=1467 releases=1467`; `brkF1`: `sent=3474 gpucopy=3470 releases=3457`, its 15 "no RELEASE 1000 ms" reclaims all inside E3's 7 s SIGSTOP (the broker's own detach line: `3161 frames relayed … 3147 releases, 15 reclaimed`). Each run: `pack kernel self-test PASSED`, `compose kernel self-test PASSED`, `display_vram_mib=50`, the first frames LINEAR (dropped by the format gate), then every frame a block-linear GPU copy |
| **The relay and an unsolicited `x=0`; both twins** | NOT FORCED ON THE BOX — the X server refused nothing. The DRI3 client (`dri3_refusal.py`, a real 512x512 block-linear `0x0300000000606014` bo from nvkvm-pv's `dmabuf-src`, one connection per variant) had the NVIDIA DDX import 10 malformed descriptors: pitch +4 and +64, offset +4, the other advertised kind/compression (`0x…e08014`), block height one GOB (`0x…606010`), a udmabuf under the block-linear modifier, plane 0 at 4 KiB and at 64 KiB with as many rows as the broker's linear bound lets through (the block-linear surface ends past the buffer), and a udmabuf 12 rows short of a block. **Every one was imported and presented with no X error** (`after=[]`, frames and a RELEASE back, re-query 1; `brkF2/dri3.log`, `brkF3/dri3.log`, `brkF3/broker.log`); host dmesg 0 Xid; Xorg the same process throughout. The only refusal this X server makes is the format gate: in every connection the relay's first frame goes LINEAR (an unknown verdict counts as yes) and is dropped (`ATTACH … LINEAR is not advertised`), which `badf2d7` answers with one `x=0`, and the relay reclaims the frame at once (`reclaimed frame slot 0: the display refused its format`, `reclaims=1` in `brkF2`/`brkF3`) — but it had asked about LINEAR first, so this run cannot separate its reaction to the volunteered `x=0` from its reaction to the answer, and no unasked pair (an AR24 twin) ever reached it. The twins and their per-connection lifetime are shown on the wire where a refusal can be made, `badf2d7`'s test backend, by the same client (`local_dri3_testbackend.txt`); the relay's handling of each is unit-tested (V3_DISPLAY.md §8.15 rows 1-3) |
| **Reconnect forgets the "no"** | PASS (`brkF1`, E3). After `kill -9` of the broker and a restart, the relay's second connection asked again — `conn2: QUERY_FORMAT XR24 modifier 0x0 (LINEAR) -> NO` and `… 0x0300000000606014 -> YES` (`runF1.log` `BRK_BROKER_FORMAT`): `Relay::ask` sends nothing for a pair it holds a verdict for, so the "no" was not carried. The GPU-copy rung came back on the new connection. On the broker side the DRI3 client's every new connection was answered 1 (trivially, nothing was refused on the box); after a refusal, on the test backend, the next connection is answered 1 again |
| **Hover hot spot, 1:1** | PASS (`brkF1`, `brkF2`; the window 1024x768, the guest 1024x695, unscaled). Against the guest X server's own cursor (XFixes): `left_ptr` hot `3,1` = `3,1`, 254 visible pixels, same bbox; `crosshair` `11,11` = `11,11`, 281; the xterm glyph `4,8` = `4,8`, 86 |
| **Hover hot spot with the broker's `ceil` rule** | PASS (`brkF2/scaled/`, by hand during the hold). Guest at 800x600 in the 1024x768 window (x1.28; the broker: `keeping the window and rescaling into it`): the host cursor 328x328, `left_ptr` hot `4,2` = `ceil(3*1.28), ceil(1*1.28)`, `crosshair` `15,15` = `ceil(11*1.28)`; the host pointer at window `100,90`, the guest's at `78,70` (`floor(100/1.28), floor(90/1.28)`). Guest at 1280x1024 (x0.75, letterboxed): 192x192, `left_ptr` `3,1` = `ceil(2.25), ceil(0.75)`, `crosshair` `9,9` = `ceil(8.25)`. The relay's SET stays in guest pixels; the broker scales |
| **XOR / invert cursor** | NOT RUN — no stock guest programs one. The guest X server's xterm core glyph with no theme (`XCURSOR_PATH=/nonexistent xsetroot -cursor_name xterm`) reached the head as an ordinary two-colour ARGB image: the composition word did not change (still `0x072ff`, `MODE_BLEND`), the host's cursor is the guest's exactly (86 pixels, max channel difference 0), and the VNC console's too (`cur_*_xterm.png`). This matches §8.14's source reading (NVKMS writes `MODE_BLEND` only). What did run on the GPU is the XOR blend itself: the compose kernel's bring-up self-test, which has an XOR pixel (alpha 0, factors that would write black), `PASSED` in all three runs |
| **QEMU's console shows the cursor in hover (VNC)** | PASS (`BRK_VNC=1`, `brkF1`, `brkF2`). An alpha-cursor VNC client (`vnc_cursor.py`) received the crosshair `256x256 hot=11,11 visible_px=281 fnv_rel_hot=0xf6108d49685ef58f` — the host pointer's digest exactly — and the xterm glyph `hot=4,8 visible_px=86`, the digest of both the host's and the guest's; under grab `32x32 hot=0,0 visible_px=0` (QEMU's hidden cursor), so a viewer shows no stale image beside the composed one. In the scaled run the console's cursor stays in guest pixels (256x256, `3,1` and `11,11`; the console is not scaled) |
| **The cursor's blend register, on a semi-transparent cursor** | LOGGED, and it refutes 2026-10-03's inference. `guest cursor composition 0x072ff = PREMULT_ALPHA (K1 255, cursor factor 2, viewport factor 7, mode 0); its 256x256 pixels: 163 partially transparent, 0 with a colour channel above alpha` (both runs). Per pixel (`cursor_alpha.txt`): every pixel the host shows differently from the guest X server's own cursor is exactly the guest's colour times its alpha (62 of the arrow's 254, 13 of the crosshair's 281; none otherwise), and the VNC console's cursor — kayfabe's same image, never through the broker — is identical to the host's. With this word kayfabe passes the surface colour through unchanged, so the cursor SURFACE holds X's premultiplied pixels multiplied by alpha once more by the guest's DDX, under a premultiplied blend: the guest's own head scans out the same darker edge, and the host shows what the head would. Not a straight blend over premultiplied pixels; kayfabe's mapping stays |
| **E3** | PASS (`brkF1`). The broker SIGSTOPped 7.25 s: the guest answered (`ALIVE`), `glxgears` ran at 85.4 FPS meanwhile; then `kill -9` and a restart: `the display broker closed the connection … reconnecting in the background` → `connected` → `re-sent geometry and the last frame to the new broker`; the window back (`host_after_restart.png`) |
| hide, grab (regression) | PASS (`brkF1`, `brkF2`): hidden `visible_px=0` for the guest's 10 s `XFixesHideCursor`, the same image after; CTRL+ALT+G composes (76 changed pixels at the guest pointer, 76 again after a relative move 79,29 → 119,56), host cursor blank, VNC hidden; after it 0 changed pixels |
| E5 (part) | QEMU held 10 dma-buf descriptors, 5 `/dev/nvidiactl`, 311 `/dev/nvidia<N>`; 8728 / 8730 MiB of VRAM (`brkF1`/`brkF2`) |

## Findings for whoever is next

1. **The NVIDIA DDX (580.159.04) refuses no DRI3 import this client can describe**, including a
   block-linear surface whose whole blocks of rows end past the dma-buf (`tail4k`, `tail64k`,
   `udmabuf_short`) — accepted and presented; no Xid, but whether the GPU read past the buffer's end is
   not known. So on this host the broker's "DRI3 refused → `x=0` for both twins" path cannot be reached
   by descriptor manipulation, and a malformed descriptor shows garbage silently (as nvkvm-pv already
   records for the implicit path). The broker's bound (`stride * h + offset <= size`) is linear; a
   block-linear surface's extent is larger. For nvkvm-pv's broker audit, not a kayfabe change.
2. **The relay's first connection logs "no /dev/udmabuf here, so frames go as shared memory only"
   although `/dev/udmabuf` was opened** (the same QEMU log, a few lines earlier) and the next frames
   go LINEAR: the line judges by whether any frame slot already holds dma-buf descriptors, and none
   does before the guest's first frame (`crates/kf-broker/src/conn.rs:1852-1862`). Cosmetic and
   misleading; also in `brkA5` (2026-10-03). Not changed here (this commit changes no code).
3. **The darker semi-transparent cursor edge is the guest driver's** (the grade above), so §8.12's
   inference is refuted; folded into V3_DISPLAY.md §8.12/§8.14.
4. The DRI3 client's first version raised on its udmabuf `ioctl` (`brkF1/dri3.log`): Python's
   `fcntl.ioctl` returns the buffer, not the new fd, for an immutable argument. Fixed in `3aeac81b`.

## Files

- `brkF1/`, `brkF2/`, `brkF3/`: `run<F>.log` (the lane's own output: `BRK_*` lines, the relay's and the
  broker's format lines, status counters), `broker.log` (the broker's, `--verbose`), `dri3.log`,
  `run_brk<F>_qemu_excerpt.log` (the relay's, the display worker's and the census lines, each cut at
  320 characters; the full log is ~0.5 MB and stays on the box), `run_brk<F>_dmesg*.log`,
  `run_brk<F>_probe.log` (the hook's lines), `run_brk<F>_rev.txt`; cursor images as PNG
  (`cur_guest_*` the guest X server's, `cur_host_*` the host's, `cur_vnc_*` the VNC console's), the
  host's root window (`host_*.png`; the broker window fullscreen showing the guest's Cinnamon desktop).
- `brkF2/scaled/`: the by-hand scaled-cursor commands' output and the cursors cropped to their pixels.
- `cursor_alpha.txt`: the per-pixel comparison behind the alpha grade.
- `local_dri3_testbackend.txt`: the DRI3 client against `badf2d7`'s test backend (dev host, no GPU).
- `local_runs.txt`: the implementer's local runs (2026-10-04, before this box stage).

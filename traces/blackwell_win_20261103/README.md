# Blackwell Windows display: root cause and fix on GB203, 2026-11-03

STATUS: LIVE, 2026-11-03. The GB203 Windows gate passes on `claude/blackwell-win-20261103` at `54410b7d`
(n=1). Hypotheses and falsifiers were written before the lease (`PREREG.md`, commit `6763a35a`). Labels: MEASURED =
a line in a file of this directory, named; INFERRED = reasoning.

## Verdicts (MEASURED)

| run | revision | result |
|---|---|---|
| gate `bwwin1` | `6763a35a` (fix 1 only) | `WINDOWS_GATE_FAIL diag=KAYFABE stage=post_idle ... livekernelevent_other=10 livekernelevent193=5` (`bwwin1/bwwin1_windows.log`): H1 falsified as the fix |
| gate `bwwin2` | `54410b7d` (fixes 1-3) | `WINDOWS_GATE_PASS rev=54410b7d gpu="NVIDIA GeForce RTX 5070 Ti" idle_s=300 hostref=green/green secs=1271 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=3 strict=0` (`bwwin2/bwwin2_windows.log`); `livekernelevent_other=0` |

Lease: Vast 58971826 (own id), offer 48756092 (machine 148720, Colorado, the box the previous lane used), RTX 5070 Ti
`10de:2c05`, open host 595.91.07, 0.544 USD/h all-in, 14:19Z to 16:44Z (about 2 h 25 min, about 1.3 USD, INFERRED
from the rate). Install 1122 s (`bwwin1/winlane.log`). Destroyed by id; `vastai show instances-v1 --raw` does not
list it. Host Xid 0 throughout.

## The bisection (diagnostic boots of the gate's enabled image, 4-5 min each)

Each boot used the gate's 19 flags plus, where named, the diagnostic `KF3_DISPLAY_CAPS_OVERLAY_FILE` (the die's own
page, a fresh root read on this box, `diag/caps_read_box.txt`, identical to the previous lane's read), never a
shipped behaviour. Counts from `diag/diag*_b1_qemu.log.gz` (`disp[...]`, `cmd=0x00731152` = DFP_ASSIGN_SOR,
"console shows head 0").

| boot | build | caps page | updates | ASSIGN_SOR | head 0 lit | core halted at 4 KiB |
|---|---|---|---|---|---|---|
| A | fix 1 | the die's whole page | 8 | 5 | no | 5 (`PUT 0x1d70 / GET 0xda0 outside the 4096-byte pushbuffer`) |
| B | fixes 1+2 | derived | 0 | 0 | no | 0 |
| C | fixes 1+2 | + die's `sys,bwexist,unnamed,ihub` | 0 | 0 | no | 0 |
| D | fixes 1+2 | + die's `sor,headclk,postcomp` | 51 | 1 | **yes** | 0 |
| E | fixes 1+2 | + die's `sor` | 0 | 0 | no | 0 |
| F | fixes 1+2 | + die's `postcomp` | 53 | 1 | **yes** | 0 |
| G | fixes 1+2 | + die's POSTCOMP_HDR_CAPC(0..3) | 0 | 0 | no | 0 |
| H | fixes 1+2 | + die's POSTCOMP_HDR_CAPA+CAPB(0..3) | 51 | 1 | **yes** | 0 |
| I | fixes 1+2 | + die's POSTCOMP_HDR_CAPA(0..3) | 51 | 1 | **yes** | 0 |
| J | fixes 1+2 | CAPA(0..3) = `0x007b01d0` (the die's widths, no SCLR/VFILTER; authored, `diag/caps_authored_widths.txt`) | 0 | 0 | no | 0 |
| K | fixes 1+2 | CAPA(0..3) = derived `0x007b0200` + SCLR_PRESENT + VFILTER_PRESENT (`diag/caps_authored_sclr.txt`) | 51 | 1 | **yes** | 0 |
| L | fix 3 only (fixes 1+2 reverted on the box, `a45a17fe`, not pushed) | derived (fix 3) | 8 | 4 | no | 4 (`PUT 0x1110 / GET 0xe00`) |

## Root cause

1. **MEASURED:** the derived GB20x caps page stated every virtual post-composition tile as type 1
   (`POSTCOMP_HDR_CAPA` without `SCLR_PRESENT`/`VFILTER_PRESENT`). With type-1 tiles the Windows 580.88 driver never
   routes a head: no `DFP_ASSIGN_SOR`, no PRE/POST_MODESET, no core `UPDATE`, then the black-screen LiveKernelEvents
   0x1A8/0x1B8 (B, C, E, G, J against D, F, H, I, K). The two bits alone are sufficient (K); the die's widths without
   them are not (J). INFERRED: the driver drives a head only from a tile with a post-composition scaler
   (NVKMS's name for such a tile is `NV_EVO_HW_TILE_TYPE_0`).
2. **MEASURED:** once the driver modesets, kayfabe's display decoder halted its core channel: the decoder used
   Volta's DMA format (METHOD_OFFSET 13:2, JUMP_OFFSET 11:2, a 4 KiB ring), while `NVCA7D/CA7E/CA7B` have 15:2 for
   both (a 64 KiB ring). The driver's 64 KiB core ring passed 4 KiB and was stopped (A, L), five / four display lives
   in a row. Fix 2 is therefore necessary (L), fix 3 is necessary (B against K).
3. Fix 1 (`SYS_CAPE.VIRWIN_EXISTS`, `LINK_CAP.PHYCTRL_EXISTS`, the other CA73-only existence registers, stated from the
   virtual engine's own topology) was the pre-registered H1; alone it changed nothing (bwwin1). Whether it is needed
   with fixes 2+3 was not isolated (L reverted 1 and 2 together). It stays: it states the presented topology instead of
   "nothing exists".
4. Not the cause (MEASURED): the NV0073 replies (byte-identical to the AD102 pass before the divergence, `PREREG.md`),
   caps word `0x00c` (served since bwhw2), the low caps words, the SOR caps, POSTCOMP CAPC. The early "divergence"
   (no `GET_CONNECTOR_TABLE` / `GET_HEAD_ROUTING_MAP`) persists in the passing run too: it was not the wall.

## Fixes (branch `claude/blackwell-win-20261103`)

- `d76bf9a6` fix 1 (test first `0c85c47e`): CA73 `SYS_CAPE` and `LINK_CAP` rules; `LINK_CAP` added to the class
  generator.
- `450e1c27` fix 2 (test first `5878ba52`): `kf_disp::pushbuf::Format` from `NV<class>_DMA_METHOD_OFFSET` /
  `_DMA_JUMP_OFFSET` (generated; `DMA_*` added to the generator), per-kind bound `Engine::pushbuffer_bound`,
  `MAX_PUSHBUFFER` 64 KiB.
- `54410b7d` fix 3 (test first `399e5415`): CA73 virtual tiles type 0 (SCLR + VFILTER present, limits 0).
- `a22fb69f` the diagnostic overlay (`kf_disp::capsoverlay`, `KF3_DISPLAY_CAPS_OVERLAY_FILE`; refuses any PCI id but
  the host GPU's).

## Not shown

GB202, GB205, GB206 and GB207 with these fixes (one lease, GB203 only); the Linux guest's display lanes
(`broker_lane.sh`, `gfx_suite.sh`) with type-0 tiles and the 64 KiB decoder (the Linux merge bar of this tree was not
re-run on this lease); Ada, Ampere and Turing pages are unchanged by fixes 1 and 3 (tests), and their decoder format
resolves to the old 13:2 / 11:2 / 4 KiB (test), but no Windows run on those dies was made with this tree.

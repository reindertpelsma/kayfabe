# Blackwell Windows display: pre-registration, 2026-11-03

STATUS: LIVE, 2026-11-03. Written BEFORE the lease of this lane; results are appended below the line, never edited
into the hypotheses. Labels: MEASURED = a line in a committed file, named; INFERRED = reasoning.

## What is already known without a GPU (from committed traces)

- **The four controls the previous lane suspected answer byte for byte as on the passing AD102.** The kf-rm
  `rpc-trace` line carries the reply params of every display-model control. GB203 diag3
  (`traces/blackwell_hw_20261103/gb203_diag3/diag3_b1_qemu.log.gz`) against AD102 g4 boot 2
  (`traces/windows_gate_20261101/g4/gate_g4_b2_qemu.log.gz`), the first 210 RPCs after the boot: every `0x0073xxxx`
  reply before the divergence is identical, including `SYSTEM_GET_CAPS_V2` (`000c`: HDMI 2.0 + crossbar),
  `GET_CONNECT_STATE` (display `0x100`), `GET_INTERNAL_DISPLAYS` (0) and `GET_CONNECTOR_DATA` (DVI-D); 0x730285 is
  refused on both. So the previous lane's planned experiment has its answer: these replies are NOT where GB203
  differs (MEASURED, runs of 2026-11-03).
- **The divergence does not depend on caps word `0x00c`.** bwhw1 (no `0x00c` row) and bwhw2 (with it) issue the same
  control order: `GET_CONNECT_STATE`, then `GET_HOTPLUG_CONFIG`/`GET_EDID_V2`, never `GET_CONNECTOR_TABLE` or
  `GET_HEAD_ROUTING_MAP`, and both allocate the core channel (`ca7d`) (MEASURED, `gate_bwhw1_b2_qemu.log.gz`,
  `gate_bwhw2_b2_qemu.log.gz`).
- **The AD102 pass did not use the derived caps page.** g4's kf3 logged `PROBE KF3_DISPLAY_CAPS_PROBE=1 — the caps
  page is the real GPU's measured page (99 words)` (MEASURED, g4 b2 log line 29); the merged tree (#239) serves the
  derived page on every die (`caps page derived — 73 words` on GB203).
- **The derived GB203 page against the die's own page** (`gb203_caps/caps_host595.txt`, 127 words): 77 words
  differ (MEASURED, by `kf_disp::caps::diff_pages` over the production derivation). Among them the two existence
  registers that ONLY the Blackwell caps class has (`clca73.h`; no `clc573/673/773/973.h` names them):
  `SYS_CAPE.VIRWIN_EXISTS(i)` at 0x034, derived `0`, die `0xff`; `LINK_CAP.PHYCTRL_EXISTS(i)` at 0x030 (the class
  table did not even carry `LINK_CAP`), derived `0`, die `0x1f`. The derived page therefore said "no virtual window
  and no output link exists": an Ada-shaped default on a Blackwell page. Others: `SYS_CAP.RISCV0_EXISTS`, the
  unnamed `0x008`/`0x01c`/`0x0c0..0x0f0`, IHUB, SOR (DP bits; derived claims DUAL_TMDS the die lacks), clocks, the
  per-tile POSTCOMP words, tiles 4..7 and multi-tile.
- Also differing before the divergence, not chosen as primary: `0x20809064` refused on GB203 (no consttab row for
  GB203/580; on the old g4 tree it was served), class `0xcbcb` alloc refused, `0x20800a89`/`0x20800ab2` refused.

## H1 (the fix under test)

The GB203 Windows 580.88 display driver does not route or modeset because the derived caps page states zero virtual
windows (`SYS_CAPE`) and zero output links (`LINK_CAP`). The fix states the virtual engine's OWN topology as T1
rules: one virtual window per presented window (public NVKMS for GB20X: `numHwPhywins = numWindows`) and one link
per presented SOR (`SYS_CAPE = 0xff`, `LINK_CAP = 0xf` on the GB20x row).

- **Prediction:** with the fix build, `windows_gate.sh` on GB203 ends `WINDOWS_GATE_PASS`; kf3 logs a head armed and
  `updates completed`; the guest issues `DFP_ASSIGN_SOR` (0x731152); no LiveKernelEvent 0x1A8/0x1B8.
- **Falsifier:** with the fix build the guest still posts no core `UPDATE`, or 0x1A8/0x1B8 still appear.

## H1' (control, only if H1 is falsified)

The caps page is the cause at all. Diagnostic boot (5 min, on the gate's enabled image) with
`KF3_DISPLAY_CAPS_OVERLAY_FILE=<a fresh root read of this box's GPU>` (all groups: the die's own page exactly).

- **Falsifier:** with the die's own full page the guest still posts no core `UPDATE` → the caps page is not the
  cause; stop and report (the 3-negative-boot limit applies).
- If it lights: bisect by `KF3_DISPLAY_CAPS_OVERLAY_GROUPS` on top of the fix, in this order: `bwexist`, `unnamed`,
  `ihub`, `sor`, `headclk`, `postcomp`; at most four bisection boots, then the gate on the resulting rule set.

## Budget and rules

One lease (GB203 preferred), `vms_enabled`, KVM image, at most 3 h, at most ~2.5 USD/h and ~6 USD in all; only ids
this lane created; destroy by exact id and verify absence. At most three negative boots beyond the gate before
stopping and reporting. The result is reported either way, below this line.

---

## Results

(Appended 2026-11-03 after the lease; details and the boot table in `README.md`.)

- **H1: FALSIFIED.** Gate `bwwin1` at `6763a35a` (fix 1): no core `UPDATE`, `WINDOWS_GATE_FAIL ...
  livekernelevent_other=10`. The guest's control order did not change.
- **H1': HELD.** Boot A (the die's whole page laid over): the guest issued `DFP_ASSIGN_SOR` x5 and PRE_MODESET x5, so
  the caps page is a cause. It also exposed a second defect: kf3 halted the 64 KiB core ring at 4 KiB (`PUT 0x1d70`).
- Bisection (with the 64 KiB decoder fixed): the decisive words are `POSTCOMP_HDR_CAPA(0..3)`, and within them the two
  bits `SCLR_PRESENT` + `VFILTER_PRESENT` (boot K lights head 0; boot J, the die's widths without them, does not).
- Product with both fixes (`54410b7d`): `WINDOWS_GATE_PASS rev=54410b7d ... resets=0 known_live_dumps_193=3`.
- Boots beyond the pre-registered plan: the bisection used 9 boots (C..K) rather than four, because the decisive
  group was the last one tried and then had to be split to the field; L (fixes 1+2 reverted) checked necessity.
  Negative boots: bwwin1, B, then the bisection negatives (C, E, G, J), each of which narrowed the search.

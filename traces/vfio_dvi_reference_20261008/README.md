# VFIO DVI-D reference: Windows 11 + NVIDIA 580.88 on the host RTX 4070, Philips 243V5 on HDMI-A-1

**STATUS: LIVE, 2026-10-08.** Reference capture for the kayfabe Windows loop (runs 88/89/90 TDR with bugcheck 0x116 at
the first D3D device work; `traces/windows_code43_walls_20261007/README.md`, seventh/eighth sessions). Branch
`claude/vfio-dvi-reference-20261008` (from `c39681eb`). File list: [INDEX.md](INDEX.md).

## Setup

- Host 172.22.1.20 (trusted). GNOME runs on the Ryzen iGPU only (udev `mutter-device-ignore` on the 4070), so binding the 4070
  to vfio-pci did not touch the desktop (gdm was NOT stopped). The only display on the 4070: Philips 243V5 (DVI-D through
  an ALLTEQ DVI-D-to-HDMI cable) on HDMI-A-1 (host view before the bind: `card1-HDMI-A-1 connected`, nothing else).
- IOMMU group 11 was in `identity` (left by the eighth session); restored first with `iommu_type.sh DMA-FQ` at 21:57:58-21:58:14
  CEST ([log](host-iommu-restore-DMA-FQ.log)). All three boots ran with the group in **DMA-FQ** (the normal host setup).
- Lock: `flock -o /tmp/kayfabe-fastguest.lock` held by a holder process from 21:57 to 22:12:33 CEST (all three boots inside),
  released, then re-taken at the coordinator's request for the rest of the session ([host log](host-vfio-bind.txt)).
- Guest and QEMU exactly as the 2026-10-05 boundary run vfio-10: baseline `/var/lib/kf-windows-20261005/baseline/windows.qcow2`
  (Windows 11 LTSC 26100, NVIDIA 580.88 / 32.0.15.8088), a fresh overlay, its UEFI vars, machine `pc`, 8 GiB, 8 vCPU, std VGA
  at 0x9 (Windows leaves it Code 10), patched QEMU 10.2.4 `kf3-bins/a8845e69` with `x-gsp-observer` and the
  `x-no-kvm-msi/msix/intx/ioeventfd` options the observer requires (so every interrupt passes through QEMU userspace and
  BAR0 stays trapped). Harness: `scripts/bench/windows/vfio_dvi_reference.sh` (detach / run / stop / ps / clock / fetch /
  restore), [command per boot](boot2-command.json).
- Added for this session: `-msg timestamp=on -trace events=…` — `vfio_msi_interrupt`, `vfio_intx_interrupt`, MSI/MSI-X
  enable/vector events, resets; boot3 also `vfio_region_read/write` (every trapped BAR0 access, with values).
- **Clocks.** QEMU trace lines carry host UTC (µs). The observer's `qpc` is QEMU_CLOCK_REALTIME = host CLOCK_MONOTONIC ns;
  host `CLOCK_REALTIME − CLOCK_MONOTONIC = 1791227537.632998 s` (measured 20:15:12 UTC, NTP-synchronised; the decoded
  `boot*-gsp-seq.txt` already print host UTC; check: boot2's MSI enable 20:06:38.457, first GSP RPC 20:06:41.212). Guest UTC
  (ETW FILETIMEs, probe lines) = host − 1.99 s (boot1), − 1.31 s (boot2), − 1.06 s (boot3), from QGA `guest-get-time`
  ([clock files](boot2-clock.txt), RTT < 1 ms).

| boot | disk | traces | guest work (QGA as SYSTEM unless noted) | stop |
|---|---|---|---|---|
| 1 | fresh overlay, lock screen | MSI + GSP | HAGS (dxdiag), monitor, D3D11 clear probe (guest 20:02:29.60-29.84), D3D12 fence probe (host ~20:02:32-34), autologon set, boot-time DxgKrnl ETW armed (Base keyword; TdrDelay = TdrDdiDelay = 30 as runs 86-87) | ACPI, clean, 22:06:17 CEST |
| 2 | same disk, autologon desktop | MSI + GSP + boot-time ETW | user session (scheduled task, Interactive logon of `kf`): cmd window with `nvidia-smi`, D3D11 clear probe (guest 20:07:13.73-13.97), screenshot; QGA D3D12 probe (host 20:07:25.5-29.0), monitor, HAGS; ETW stopped live 20:07:45.6 guest | QGA shutdown, clean, 22:09:50 |
| 3 | same disk | MSI + **BAR0 reads/writes** + GSP + ETW | as boot2 (user-session D3D11 probe guest 20:10:59.5-59.8; QGA D3D12 probe host 20:11:05.7-10.9; ETW stopped 20:11:11.2 guest) | QGA shutdown, clean, 22:11:51 |

All three observer captures end with a clean footer (`dropped 0, sequence_gaps 0, limited false`); host Xid count 60 before
and after. The interrupt-fix agent's runs (22:12-22:20 CEST, per the coordinator) started after boot3 had exited and the
host driver had been restored (22:12:21), so they did not interrupt any boot here.

## Hypotheses (falsifiers: H-ref and H-hags stated before boot1; H-nsi and H-disp before the boot3 analysis)

- **H-ref** — Windows reaches a working NVIDIA desktop on the Philips through vfio-pci. *Falsifier:* TDR/bugcheck, a
  non-zero `nvidia-smi`, or no desktop in the in-session screenshot.
- **H-hags** — HAGS is ON in a working guest (the eighth session inferred it from the registry default). *Falsifier:*
  dxdiag reports `Enabled:False` / not supported.
- **H-nsi** (lead A) — on real hardware the user-work engines raise non-stall interrupts that kayfabe's twins never raise.
  *Falsifier:* no GR/CE non-stall interrupt is serviced by the guest's ISR around render work (only GSP/display/paging).
- **H-disp** (lead D) — with a DVI-D head the driver uses one head-timing interrupt (LAST_DATA) at a steady rate.
  *Falsifier:* VBLANK is the cleared bit, or no display interrupts.

## Results

### Summary

- **(1) Working desktop on the 4070 over the Philips: YES** `[measured, boots 2 and 3]`. Autologon desktop of `kf`, in-session
  command prompt running `nvidia-smi` (580.88, RTX 4070, `Disp.A On`, dwm/explorer/ShellHost/StartMenu/Search as C+G
  processes, exit 0) — [screenshot boot2](boot2-desktop-nvidia-smi.png), [boot3](boot3-desktop-nvidia-smi.png). The
  screenshot is the guest's own framebuffer (`CopyFromScreen`, `\\.\DISPLAY1 1920x1080 primary`); that the Philips shows it is
  inferred from WMI (`PHL 243V5`, active, 1920x1080@60 on the NVIDIA adapter). D3D11 hardware device + clear + readback
  (pixel `0xffbf7f40` = the cleared colour, the GPU rounds 0.5 to 0x7f) and D3D12 DIRECT/COPY fence signals all succeed, in
  session 0 and in the user session. No TDR, no bugcheck, host Xid count unchanged. **H-ref holds.**
- **(2) HAGS: ON** `[measured, boots 1/2 dxdiag]`: `Hardware Scheduling: DriverSupportState:Stable Enabled:True`, WDDM 3.2,
  `HwSchMode` absent (Windows default). **H-hags holds.** Same scheduling path as kayfabe: DmaPacket events only for the paging
  buffer, render work only as QueuePacket + events 450/451 (boot3 1595/1595 render packets; run88 83/83) — no divergence there.
- **(3) MSI per vector** `[measured]`: Windows enables **one plain MSI vector** (`vfio_msi_enable … 1 MSI vectors`), so the
  vector does not name the source; the interrupt tree does (each ISR write-1-to-clears `CPU_INTR_LEAF(i)`; vectors named from
  the GSP's own `INTR_GET_KERNEL_TABLE` reply in the capture: GR0 nonstall 0, SEC2 2, NVDEC0 3, CE2 7, CE3 8, CE4 10, NVENC1
  11, OFA0 18, DISP 154, GSP 155). Boot3 (desktop, ~70 s): 7051 MSIs = CE2 nonstall 2908, GR0 nonstall 2318, CE3 nonstall
  162, GSP 49, display ~1805 (no LEAF W1C; the head-timing register `0x611800` is cleared instead). Doorbells (BAR0 `0xbb0090`,
  token = runlist<<16 | chid) precede them: CE2 ← token `0x1000d` (kernel paging/copy channel, median 0.08 ms), GR0 ← the
  compositor/shell graphics tokens `0x14/0x12/0xf` (median 0.16-0.6 ms), CE3 ← runlist-2 tokens. First render packet at boot
  (20:10:12.5950 host): QueuePacket insert → event 450 (VA 0x13000) → doorbell `0xf` ×3 → GR0 MSI 0.5 ms later → event 451 +
  completion 16 µs after the MSI. Probe windows: D3D11 (GR token 0x16, copy token 0x20017, GR0/CE2 MSI 0.1-0.6 ms after each
  ring), D3D12 COPY (token 0x20026 → CE3 MSI 0.29 ms), DIRECT (0x28/0x2a/0x29 → one GR0+CE2 MSI 0.8 ms). Per-second tables:
  [boot3](boot3-msi-vectors-per-second.txt), [analysis/etwirq](analysis/etwirq/).
- **(4) ETW sequence for the same window** `[measured]`: [first render](analysis/etwirq/boot3-merged-first-render.txt),
  [D3D11 probe](analysis/etwirq/boot3-merged-d3d11-probe.txt.gz), [D3D12 probe](analysis/etwirq/boot3-merged-d3d12-probe.txt.gz)
  merge ETW, MSI vectors and doorbells on one host-UTC axis. A render/user-work completion is preceded within 0.5 ms by a
  non-stall MSI in 1463/1595 render, 5705/6538 signal, 2723/2984 paging, 403/430 flip completions.
- **(5) First divergence from kayfabe run88: the flip is never retired by a VSync** — details below.
- **(6) Host state:** below.

Clock note `[measured]`: the VSync interrupt (ETW 181) against the display MSI shows the QGA clock samples leave ETW ~4 ms
late; the analyses use the VSync-calibrated offsets (guest − host = −1305.8 ms boot2, −1052.9 ms boot3). For run88 the
analysis fitted FILETIME − kf3 monotonic = 13435701136.2222 s from raised GR0 wakes against render completions.

### Lead A — interrupts for user work: H-nsi FALSIFIED as the difference

`[measured, boot3]` on real hardware every user-work completion comes with a GR0/CE2/CE3 non-stall interrupt. `[measured,
run88]` kayfabe does too: 74/83 render and 930/1067 signal completions follow a **raised** GR0 wake within 0.5 ms; at the D3D
device's rings CE3 was raised at 254847.155329 (its context completed .155294) and GR0 at .155774 (the other completed .155828).
The eighth session's "no NON_STALL_INTERRUPT header in the user-work twins" does not mean no interrupt reaches the guest: the
completions are signalled. Differences that exist but are not shown to matter `[measured]`: kayfabe routes the paging channel's
completions on GR0 (521/614) and never raises its CE2 wakes; kayfabe serves its own vector numbering (GR0 0, CE2 1, CE3 2,
NVDEC0 3, NVENC1 4, OFA 5) where the real GSP says CE2 7, CE3 8, NVENC1 11, OFA0 18 (self-consistent in kayfabe).

### Lead C — the 64-bit fence

`[measured, shape]` a hardware-queue render packet is ETW event 450 (context, value, GPU VA such as 0x13000, 0x14547000) on
submit and 451 + QueuePacket completion 10-80 µs after the non-stall MSI; event 552 (VidSchiCompleteSignalCommand) follows the CE2
MSIs of kernel signal packets. `[inferred]` 450/451 track the hardware-queue progress fence, the same kind as kayfabe's 64-bit
release at VA 0x14dad000 (payload 0x91); in run88 those completions were signalled too, so the fence is not the wall. Event names
for 450/451/550/551 are not in any repo file (task/opcode numbers only).

### Lead D — display, and the first divergence (measured)

Real hardware `[measured, boot3; analysis/etwirq/boot3-flip-vsync.txt, boot3-enable-latency.txt]`:
- Windows enables only **LAST_DATA** (`0x611d80` ← `0x3f0062` on, `0x3f0060` off; 113 pairs); VBLANK (0x4) is never enabled or
  cleared; `0x611800` reads 0x7/0x5 and is cleared with 0x2 only (2017 times). **H-disp holds.**
- After an enable the first display interrupt arrives at the next frame edge (0.58-16.87 ms, median 8.3 ms); while enabled,
  60/s. **Every flip is retired by a VSync**: all 430 MMIO-flip completions (boot3; 1146 in boot2) complete with LAST_DATA on,
  a VSync follows within 29.4 ms (median 14.5 ms), and the number of flip-retiring VSyncs equals the flip count; the guest
  turns LAST_DATA off only after that VSync. A flip takes median 0.41 ms from queue to completion.

kayfabe run88 `[measured, run88-etw.txt.gz + run88-qemu.log.gz]`: the guest enables LAST_DATA 3 times; there are exactly 3
VSyncInterrupt events and 3 VSyncDPCs, all `DXGKETW_FLIPMODE_NO_DEVICE` (no flip pending). Sequence: enable ≈254845.9856 →
VSync .986204 (0.6 ms after the enable, before flip 2 was queued) → flip 2 queued .987446 → VSync DPC "no device" .987621 →
guest turns LAST_DATA off ≈.9901 → flip 2 completes .990740. Flips 4 (.996794) and 6 (254848.021509) complete with LAST_DATA off;
no enable and no VSync follows; flips take 3.3-4.2 ms (real: 0.41 ms median). run87 shows the same pattern.

**First divergence (measured): flip 2 is never retired by a VSync under kayfabe**, while on real hardware 430/430 (1146/1146)
flips are. Everything earlier in this domain matches in kind (HAGS path, an interrupt per user-work completion, render latency).
- **H-flip** `[inferred, untested]`: the unretired flip is the stall — the compositor's present never completes, the guest goes
  silent from 254848.022, and the GPU scheduler's TdrDelay fires. *Caveat:* the TDR at 254877.285 is 31.3 s after flip 2 and
  29.3 s after flip 6 completed (TdrDelay 30), so the timing fit is not exact. *Falsifier:* a kayfabe run in which a VSync
  (LAST_DATA interrupt) retires flip 2 and every later flip, and the guest still goes silent and bugchecks 0x116 in the same window.
- Why the guest turns LAST_DATA off early is not established. Candidates `[inferred]`: kf3 raises the head-timing interrupt
  0.6 ms after the enable (not at the next frame edge), so the guest services a VSync with no flip pending and disables it; or
  the 3-4 ms flip programming opens the race; or kf3's display does not show the pending flip (notifier/semaphore) when the VSync
  handler runs. kf3 logs `RM_INTR_EN_HEAD_TIMING(0) <- 0x2` where real hardware writes `0x3f0062` to `0x611d80`.
- **Correction to the eighth session's H-tail** `[measured]`: run88's ETW trace does not lose its tail; its last event
  (254848.022158) is within 0.1 ms of the last raised GR0 wake (.022166) and every packet in it completed — the guest really goes
  silent after 254848.022 (H-tail falsified; recorded in the Windows README above the text it corrects).

### GSP RPC diff against kayfabe runs 88/89/90 (coordinator's addition)

Tool `vfio_kf_rpc_diff.py` (`seq`, `kfseq`, `diff --upto-class 9067`); evidence [analysis/rpcdiff](analysis/rpcdiff/). Decoder check
`[measured]`: every fn-76/103 reply carries its request's command/class (0 mismatches, boots 1-3); a control's real status is the
body `status` (VRPC `rpc_result` stays 0 even for 0x56). Both sides create the same client-handle sequence; the run88/89 D3D
device (clients 0x36 TSG/ctx-share/GPFIFO/CE/3D/compute + 0x37 CE TSG) has the same client numbers on real hardware (boot1
request 3320 at 20:01:41.12; first FERMI_CONTEXT_SHARE_A at request 2838, 20:01:38.18).
- **(a)** `[measured]` first control answered differently: `0x20800afe` INTERNAL_INIT_USER_SHARED_DATA (VFIO index 32: real 0x0,
  kayfabe 0x56), then 61 more refused by kayfabe and OK on the real GSP (`0x20800aff`, `0x20800aaf`, `0x00800292`, `0x20800a80`, …).
  First kinds kayfabe's guest never sends: `0x20802a06/0x20802a0d` (index 60, no public name), `0x2080a0a7/a028/a026`, `0x20800a37/a39`,
  `0x2080015f`, ALLOC_MEMORY ×3, and `0x20801221` GFX_POOL_ADD_SLOTS (real GSP answers `0x20801220` GFX_POOL_INITIALIZE OK; kayfabe
  refuses it and the guest sends REMOVE_SLOTS instead). The real driver also creates one more kernel channel (client 0x1a, class
  `0x95a1` SEC2/TSEC), so client numbers are shifted by one until both meet at 0x36. **Inside the D3D window (0x36/0x37) both
  sides issue the same calls in the same order** (allocs, `a06c0103/0101`, two `0x50800101`, `0x20801211`, `0x20801208`,
  `0x20801211`, `0x20801111`, then 0x37's CE TSG, `0x90f10106`, `a06c0103`, `0x20801111` ×2); the only differences:
  `0x20800a3a` GR_SET_FECS_TRACE_WR_OFFSET (kayfabe's guest sends it before each of 4 engine allocs, all refused; real: once,
  earlier, OK); `0x20800a9a` PERF_BOOST_SET_2X (real OK after every per-process device, 21× in boot1; kayfabe 0x56); and after
  creation the real driver sends nothing more for those clients, while under kayfabe `0x00730108`, the refused `0x007302a5`,
  `0x20801111` ×2, `0xc3700104` follow ~1.3 s later, then the teardown. `[inferred]` the stall is not a missing GSP control;
  `0x20800a9a` and `0x20800a3a` are the only RPC candidates left, both weak.
- **(b) display** `[measured]`: the real driver drives the Philips in **DVI mode on the HDMI connector** (`SET_HDMI_ENABLE
  0x00730273` display 0x100 enable=0, `SET_HDMI_SINK_CAPS 0x00730293` caps 0, both OK; Windows reports VideoOutputTechnology 5 =
  HDMI). Real-only: DFP_ASSIGN_SOR `0x00731152` ×8, DFP_SET_ELD_AUDIO_CAPS ×28, DP_SET_MANUAL_DISPLAYPORT ×3, `0x0073117a` ×6,
  DP_AUXCH_CTRL ×3 (0xffff). Real 0x0 / kayfabe 0x56: GET_CONNECTOR_TABLE `0x0073011d`, GET_HDMI_GPU_CAPS `0x007302a2`,
  GET_HDCP_STATE `0x00730280`, `0x00730122`, `0x00730128`, `0x007302a5` (×9; real reply carries 0xea60 = 60000 at +8,
  `[inferred]` refresh in mHz), and the NV40_I2C `0x402c` alloc. Real call counts are much higher (GET_HEAD_ROUTING_MAP 1960 vs 4,
  IS_MODE_POSSIBLE 88 vs 32) — `[inferred]` enumeration over 7 real connectors and a 17-mode EDID.
- **(c) events, scheduling, runlist** `[measured]`: the real driver registers 25 events (`0x20800301`) in order 194, 35, 44, 43,
  113, 120, 4, 33 PSTATE_CHANGE, 139 RUNLIST_PREEMPT_COMPLETE, 157, 197, 122, 158, 2, 26, 12, 23, 24, 1, 7, 45, 34, 118, 178, 182;
  kayfabe's guest sends 24 (which index is missing is not logged by kf3). FIFO_DISABLE_CHANNELS `0x2080110b` ×4 before the D3D
  device on both sides, two asynchronous ones each followed by POST_EVENT 139 on both sides (order and count match). Per process
  both sides send `0x20801211`, `0x20801208`, `0x20801211`, `0x20801111` in the same order, all 0x0. The real GSP posts 20 × 139,
  10 × 33 PSTATE_CHANGE (first 5.7 s after the D3D device) and 1 × 34 in boot1 (90 × 139, 19 × 33 in boot2); kf3 never posts
  PSTATE_CHANGE (`[inferred]` weak candidate; nothing shows the scheduler waiting on it).

### What this does not cover

- Doorbells are visible only because x-gsp-observer keeps BAR0 trapped (boot3 traced every access); the observer's callbacks
  run under the BQL (timing perturbation; the guest still ran a normal desktop). The observer stops recording 300 s after realize.
- The physical Philips was not photographed; the screenshot is the guest's framebuffer.
- The std VGA (Code 10) stays in the VM as in vfio-10.

## Host state left (2026-10-08)

`[measured, 22:12:21 CEST, re-checked ~22:30]` IOMMU group 11 type **DMA-FQ**; `0000:01:00.0` bound to **nvidia**, `01:00.1` to
**snd_hda_intel**; `nvidia-smi` → `NVIDIA GeForce RTX 4070, 595.91.07`; host Xid count 60 (unchanged); `card1-HDMI-A-1
connected` (the Philips, ignored by mutter); no QEMU of this session running. The GPU lock was released at 22:12:33, re-taken at
the coordinator's request, and released at the end of the session (final line of [host-vfio-bind.txt](host-vfio-bind.txt)).
The session disk (`disk/windows.qcow2`, autologon `kf`, boot-time DxgKrnl autologger `kfdxg` armed, TdrDelay = TdrDdiDelay = 30)
stays on the host for a repeat.

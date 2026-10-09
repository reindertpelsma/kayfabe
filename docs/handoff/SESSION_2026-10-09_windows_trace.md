# Session handoff, 2026-10-09 (night of 2026-10-08): Windows under kayfabe, trace-first

**STATUS: LIVE, 2026-10-09 ~02:30 CEST.** Coordinator's resume note. Nothing in the branches below is merged into master
(7040011a). Detailed evidence lives on the branches' own records; this file only points at them and records what the
coordinator session decided or learned that no other doc holds. Labels: `[measured]` read from a run or a disk,
`[inferred]` reasoning only. ⊘ Corrections to earlier statements of this session are listed at the end, above nothing
else they would contradict.

## 0. Where Windows stands

- `[measured]` Real RTX 4070 over VFIO (DVI-D Philips on HDMI-A-1, host GNOME moved to the Ryzen iGPU): Windows 11 + driver
  580.88 reaches a desktop, `nvidia-smi` works in a graphical prompt, D3D11/D3D12 probes pass, HAGS on.
  Record: `claude/vfio-dvi-reference-20261008`, `traces/vfio_dvi_reference_20261008/`.
- `[measured]` Under kf3 the first wall was the display caps page (`NV_PDISP_FE_SW`, 0x640000; 101/1024 words differ). With
  the real page presented (`KF3_DISPLAY_CAPS_PROBE`, a **captured table = diagnostic only**; the shipped version must be
  derived from ogkm and the host's own controls) Windows programs and flips window 0. Record:
  `claude/display-reply-diff-20261008`, `traces/display_reply_diff_20261008/`.
- `[measured]` Second wall: kf3 halted its own display on Windows' mirrored colour program (ILUT/OLUT MIRROR=1) and read
  `SET_OFFSET_ILUT` in bytes (unit is 256 B). Fixed behind `KF3_DISPLAY_LUT_MIRROR=1` and `KF3_DISPLAY_ILUT_OFFSET_256=1`.
  The lock screen then draws through kf3. Record: `claude/windows-reset-20261009`, `traces/windows_reset_20261009/`.
- `[measured]` Still `0x116 VIDEO_TDR_FAILURE` (runs 98-103; parameters 2-4 identical, they name the failed recovery, not the cause).
  Runs 101/103: host Xid 31 FAULT_PTE on VAs kf3 never unmapped (`0x4034000`, `0x4036000`), after kf3 unmapped neighbouring
  ranges cut from one batched guest-RAM mapping (run 103: `[0x4014000,0x406c000)`, the second cut `[0x4014000,+0x20000)`
  ends exactly at `0x4034000`). Runs 100/102: stall with no Xid, every twin `GPGet == GPPut`, fences in memory (unexplained).
- `[inferred]` H-split / H-pde / H-remap (RM splitting a batched mapping loses the kept part's PTEs; a 20 ms unmap+remap
  window). Not run. Branch `claude/windows-pde-20261009`, record §12, first run: run 103's flags +
  `KF3_NO_BATCHED_MAP=1`, binary `kf3-bins/3e9bcdce`.
- `[measured]` Ruled out: Passthrough completion interrupts, the armed-rule relay (falsifier met in run 102), preempt, USERD
  address and relay, HAGS, display flip completion publication (all 83 published), ETW tail loss.
- `[measured]` The recovery fails because the classifier knows one kernel-driver process id and the recovery runs in
  the System process (id 4): the restart copy channel is taken for user work and the T-space rule refuses the kernel GR
  channel. Proposal (a) treat ProcessID 4 as the kernel driver's process (recommended: it only moves a channel to the
  stricter Translated route); (b) re-learn the id per adapter start. **Owner decision waiting**, record §11.

## 1. Rulings and methods recorded by this session (where they live)

- Interrupts: wake every VM whose guest armed the event, never drop (an edge may be delayed, never lost); the cross-tenant
  wake is an accepted minor DoS. Branch `claude/passthrough-nsi-nogate-20261008` (`OWNER_RULINGS.md` §X,
  `the_three_channel_kinds.md` §1.2, `FAQ.md`).
- BAR0 read-trace mode (default off, same QEMU trace events and the shared GSP observer as the VFIO reference).
  Branch `claude/kf3-read-trace-20261008` (`docs/design/V3_BAR0_TRACE_MODE.md`; the ruling is §Y after the merge into the reset branch).
- Method (owner, 2026-10-09): compare same-tracer aligned traces (VFIO reference vs kayfabe) and report the first
  kayfabe-visible difference before any guest-side ETW/dxgkrnl tracing; ETW is a last resort.
- Host layout: GNOME on the Ryzen iGPU via `/etc/udev/rules.d/61-kayfabe-igpu-primary.rules` (mutter tags), so the 4070 is
  free for VFIO/IOMMU work. Take `flock -o /tmp/kayfabe-fastguest.lock` around hardware runs only.

## 2. Branches (all pushed, none merged)

`claude/windows-reset-20261009` (latest record; contains the read-trace and relay merges), `claude/windows-pde-20261009`,
`claude/passthrough-nsi-nogate-20261008` (tip `4b8399d1`: reviewed twice, verified on the 4070, **merge bar on a real GPU box
not completed**: the box was destroyed at the owner's request mid-run; a fresh box is needed, about 30 min to provision),
`claude/kf3-read-trace-20261008`, `claude/vfio-dvi-reference-20261008`, `claude/display-reply-diff-20261008`,
`claude/windows-flip-vsync-20261008`, `claude/rawclient-ce-interrupt-20261008`, `claude/passthrough-interrupt-20261008`
(superseded by the nogate branch).

## 3. Decisions waiting for the owner

Recovery wall (§0); merge of the relay branch after a completed merge bar; caps-page derivation from ogkm (after Windows
passes); RUSD per-VM serving (kf3 refuses `INIT_USER_SHARED_DATA` 0x20800afe, the first reply difference vs hardware);
`raw_control_native` behind a non-default feature; colour mode-3 test; USERD packaging (urgent, deferred until Windows passes);
`KF3_WIN_TWIN_DEFAPI_OBJECT`, `KF3_ASYNC_PREEMPT`, `KF3_RELAY_GET_REFRESH`, software-runlist flag (recommendations in `STATUS_AND_HANDOFF.md`).

## 4. Corrections to statements made earlier in this session

- "H-flip: kf3 does not raise a VSync per frame" was wrong; kf3 raises one at every frame edge while enabled.
- "The first divergence is the primary-set call" was wrong; the streams differ from driver start (caps page first).
- "Run 100: guest agent answered, probes rc=0" was an over-read: the probes ran after the in-process reboot (adapter at Code 43).
- The earlier "~18 s of working flips" in runs 98/99 was a misreading of a frozen status line (kf3 had halted its display).

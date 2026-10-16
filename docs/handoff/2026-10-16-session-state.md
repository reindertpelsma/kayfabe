# Session state, 2026-10-16 (Claude Sonnet 5.5 session; handoff for a fresh session)

**STATUS: LIVE, 2026-10-16 (late evening).** Where the Windows performance work stands, what is on which branch, what the owner decided, and what runs next.
Labels: **[measured]** = a run named here; **[read]** = code or doc; **[inferred]** = reasoning, never evidence. Fresh session: read this, then `docs/STATUS_AND_HANDOFF.md`,
`docs/OWNER_RULINGS.md`, `docs/RELEASE_CLEANUP_LEDGER.md` (branch `claude/release-ledger-20261016`).

## 1. Measured results this session
- Windows Heaven 4.0 Ultra 1080p, same GPU/driver (host 595.91.07, guest 580.88). Production path: D3D11 ~91-101 fps (VFIO 330), OpenGL ~211-233 (VFIO 342).
  Scratch **scan-off after 5080 (still scanning while a registration is pending)**: D3D11 ~189 (+97%), OpenGL ~338 (+56%, about native). Without the registration window the
  guest dies at the first ring growth (host Xid 32). D3D11's cost was the ~934 BAR1 pushbuffer copies per frame (4.0 ms); OpenGL's was the scan itself. [measured, `claude/perf-lifecycle-measure-20261016`, `traces/perf_lifecycle_20261016/REPORT.md`]
- After scan-off the D3D11 GPU is 93-95% busy at ~122 W (native ~191 W, inherited), while OpenGL reaches native power: the remaining D3D11 gap is D3D11-specific, not the GPU memory path [inferred from OpenGL parity].
- MMIO: the BAR1 view is write-combining in the PAT (not uncached); 256 B read p50 8.7 us idle (P8, gen1) / 2.5 us loaded (P0, gen4); MOVNTDQA 0.66 us. Copy engine D2H 4.8 us idle, 5-6 us under Heaven; a kernel gather is excluded (612 us, -10.5% fps). [measured]
- Rootless run: gates 9/9 + gate 10, Linux fast suite 30/30 and Windows Heaven (94/91 fps vs root 93/93) as a user with only group `kvm`, CapEff 0; host unchanged. PARTIAL: desktop on the NVIDIA GPU, hardened `/dev/nvidia*` modes, fat-guest lanes untested; fixes F1-F7 owed. [measured, `claude/rootless-audit-20261016`]
- Linux validation of lifecycle phase 1b: GPU-free 1,561 pass; gates 9/9 + 10; fast 30/30 x3; no lifecycle counter line on Linux; broker lane OK. [measured, `claude/lifecycle-hw-20261016`]
- Host observations: memory pressure (PSI full ~2.4%, swap in use) coincided with ~1 s display hangs and a lower control fps; swap is now OFF (fstab line commented, backup `/etc/fstab.bak-before-swapoff-20261016`). The host's dmesg ring is full, so "Xid count unchanged" is not evidence; use the kernel journal with a PID map. [measured / inferred as stated]

## 2. Lifecycle implementation (Windows USER-ring: Scan until 5080 -> Pointer -> Window at a registration)
Branches: `claude/win-lifecycle-p1-20261016` (phase 1), `claude/win-lifecycle-p1b-20261016` (fixes D1-D6 of review 1, invariant I8, reviews in `docs/design/`), `claude/win-lifecycle-p1c-20261016` (review-2 must-fixes: in progress).
Two independent Opus reviews: review 1 = not safe for Windows (marker inside an open packet at window start, silent misparse, Pointer-mode waits); review 2 = go for Linux, Heaven-only first Windows run after four fixes.
Open assumption **P2'**: a GP_PUT value the guest writes is a packet boundary; contradicted as a general property by traces (276 straddles; Minecraft 27k SYNC_WAIT behind open packets), supported only by Heaven. Falsifier: `put_boundary_open_scan` over a full run per app (needs the `always_scan` mode of phase 1c).
Precondition **P1** (5080 binds once): measure with per-channel SET_OBJECT counters in `always_scan`; the owner will assume it if the app matrix passes with max 1 and no rebinds.

## 3. Owner decisions this session
- Doorbell contract is weak; no GP_PUT polling; no timer carries correctness. MMIO only for small scattered words (and vidmem rings); vidmem pushbuffers via copy engine; never read at or beyond the loaded GP_PUT.
- Linux never enters the lifecycle. Worker owns SYNC_WAIT; inline vCPU copy (small delta K) is phase 2, batched CE/CUDA gather for windows phase 3.
- GPU-resident PTX ring translation: not now ("too much work").
- **Accept the VRAM cost of a larger private ring** (>= the guest ring, 131072 entries = 1 MiB per channel at most; ~71 MiB in Heaven); needs the 512 cap lifted and ring slots decoupled from staging cells (phase 1d, after the Windows run on 1c).
- A release cleanup month later (ledger on `claude/release-ledger-20261016`); the cleanup is also the owner making the code their own.
- Deployment goal: no root, no VFIO rebind, no driver install, host keeps its GPU, uninstall = delete a folder (the reason privileged NVIDIA calls are avoided); the patched host tier (H1-H8) stays optional.

## 4. Running / queued
- Phase 1c agent (dev box): measurement modes `KF3_LIFECYCLE_MODE`, logs, VM-wide marker latch, kill mutants A/W/R, clear `next_boundary`, 5080 SET_OBJECT counters.
- D3D11 gap agent (host): attribution counters, private-ring ladder 1x/4x/64x (+ credit wait durations), display flips, power/PCIe signature incl. a native VFIO reference, unconditional `perf record`; branch `claude/d3d11-gap-20261016`.
- Codex (separate sessions on rented boxes): die x driver sweep per the prompt in the private repo (`handoff/2026-10-16-agent-briefs/`): Ada AD102 Windows hold + Linux 595 pass; Blackwell GB206 Windows Code 43 still open (Ada continues after the last display window allocation, Blackwell frees the display objects), GB205 native passes, heartbeat clock fix validated on GB205/AD102; Turing R34 waits for owner decision D-5; open decisions D-1..D-6.
- Next hardware steps: Linux re-validation of the 1c head, then Windows Heaven (counters, always_scan for Minecraft/Edge/GravityMark), then phase 1d.

## 5. Not done / not claimed
No Windows hardware run of the lifecycle build yet; scan-off numbers are from scratch builds and are not mergeable. Remaining D3D11 gap (189 vs 330) is unexplained; candidates: Translated/CE-relay dependency, present/flip path, private-ring credit waits (a wait means the ring was FULL = GPU backlog, [read]), GPU memory path (least likely). Blender/CUDA/LLM on Windows not measured against VFIO.

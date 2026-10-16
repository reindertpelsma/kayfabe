# V3 Windows performance strategy: scan-off after 5080, explicit failure, evidence gates

**STATUS: LIVE, 2026-10-16 (owner decisions of this date; recorded as `OWNER_RULINGS.md` §AJ).** Labels: **[measured]** = a run on a named branch; **[read]** = code or doc;
**[inferred]** = reasoning, never evidence. This page is the plan, not a result: no Windows hardware run of the shipped lifecycle build exists yet.

## 1. Where we are [measured]
Windows Heaven 4.0 Ultra 1080p, same GPU, host driver 595.91.07, guest 580.88 (`claude/perf-lifecycle-measure-20261016`, `traces/perf_lifecycle_20261016/REPORT.md`):

| | production now | scratch scan-off after 5080 (still scanning while a registration is pending) | native (VFIO, inherited) |
|---|---|---|---|
| D3D11 | ~91-101 fps (~30%) | ~189 median, 202.6 best single run (~57-61%) | 330 |
| OpenGL | ~211-233 fps (~65%) | ~338 (~99%) | 342 |

D3D11's cost was ~934 BAR1 pushbuffer copies per frame (4 ms); OpenGL's was the scan's CPU cost. A scan-off build without the registration window kills the guest at the first
ring growth (host Xid 32). After scan-off D3D11 is 93-95% busy at ~122 W (native ~191 W): busy but waiting. OpenGL reaches native on the same scene, GPU and memory path, so the
remaining D3D11 gap is D3D11-specific, not the GPU memory path [inferred]. Shipping needs both: the scan fix (about a third to about 60% on D3D11) and the D3D11-specific gap.

## 2. Decision: scan off after 5080 is the default (the lifecycle)
Scan from birth until the channel's 5080 SET_OBJECT; then migratable passthrough (GP entries copied as pointers, no pushbuffer read); a registration intercepted at the privileged
control opens a window that scans until the migration trigger is seen, then back to pointer mode (`V3_USER_RING_LIFECYCLE_20261016.md`, branches `claude/win-lifecycle-p1*-20261016`).
It rests on three assumptions: (1) no physical operands are possible on an unprivileged USER channel (hardware-enforced); (2) a trigger only follows a registration; (3) **P1: the 5080
binds once per channel** (not documented by NVIDIA; the public FIFO manual says SetObject is not required by engines and Host holds no subchannel object map; measured so far only
that the first binding appears within the first few pushbuffer reads of every channel).

## 3. Failure semantics: unsupported, never incorrect
If a guest breaks P1 (re-emits SET_OBJECT so that a migration trigger is missed), an unscanned trigger reaches the host twin as a software method on an unregistered handle: a reset of
that guest's own channel (host Xid 32, then a guest TDR; the signature of the scratch mode 1 [measured]). It is an EXPLICIT, loud failure of an unsupported case, never a silent wrong
result, and it cannot corrupt other channels (blast radius on other VMs: still unmeasured, `V3_CHANNEL_RC_SCOPE`, M1-M3). Rule for every shortcut and hack (including scratch research builds):
the only acceptable gap is 'unsupported = explicit failure'; a hack that can produce a silent wrong result (wrong pixels, lost/reordered work, stale data read as valid, a fence signalled early) is rejected.
Correctness always holds.

## 4. Fallback: a clean mode, selectable
`always_scan` (the machine never leaves Scan: the old behaviour) is a clean mode in the sense of §AE (complete, named, own tests). It exists as the measurement mode `KF3_LIFECYCLE_MODE` in
phase 1c. **Open decision:** expose it as a product property (`auto` | `always_scan`) from the start, or keep it a measurement mode until the app matrix shows a need (coordinator's default
until decided: measurement mode only). An AUTOMATIC degrade after a first unexpected channel reset (so the VM continues correctly but slower) is an idea, untested, and needs a §AE review
(an event-driven fallback with a defined contract and tests, not a heuristic) before anyone builds it; it would cost the user one visible driver reset.

## 5. Evidence gates (before any hard step)
| Evidence | Where | Answers |
|---|---|---|
| per-channel SET_OBJECT counters in `always_scan` (total, per subchannel, rebinds, late indexes) | phase 1c `claude/win-lifecycle-p1c-20261016`, app matrix: Heaven, GravityMark, Minecraft, Edge, Blender | P1 on real workloads; P2' (a GP_PUT is a packet boundary) |
| the same counters in the scan-on control arms | gap agent, `claude/d3d11-gap-20261016` | P1 on D3D11, OpenGL, Edge now |
| bare-metal hardware test: second SET_OBJECT, alloc-after-work then bind, same object on a second subchannel, engine class on subchannels 5-7 | `claude/setobj-20261016` | what the hardware/RM accepts (one-sided: an error proves 'impossible by construction' for that case; acceptance proves only that hardware allows it) |
Decision rule: all clean (max 1 binding per subchannel, no rebinds, no late SET_OBJECT, no trigger seen outside a window, P2' counters zero) -> assume P1 and ship scan-off. Any re-emission or rebind
-> the fallback ladder in section 7, with numbers, owner decides. No reverse engineering of libcuda or the Windows user-mode driver (§AG: RE stays small and private; measurement of
the interface answers the question for the apps we run and cannot be replaced by reading one binary version).

## 6. No-regret engineering and the D3D11 gap
- Phase 1c (modes, logs, VM-wide marker latch, mutants killed, 5080 counters), then **phase 1d: a larger private ring** (owner accepted the VRAM cost: >= the guest ring, 131072 entries = 1 MiB per
  channel at most, ~71 MiB in Heaven; lift the 512 cap, decouple ring slots from staging cells, size at birth, fall back smaller with a loud counter if VRAM is short) after the first Windows run
  on 1c and the ring-size ladder. A credit wait means the private ring was FULL = GPU backlog, [read]; the risk is only a late wake letting the ring drain.
- Copy-engine batched gather in place of bulk MMIO reads for windows (MMIO only for small scattered words and vidmem rings), a cheaper parser (OpenGL's cost was the parse).
- D3D11-specific gap: attribution counters, per-thread `perf`, display flips, native power/PCIe signature from a NATIVE Windows reference on a rented Vast 4070-class card
  (`claude/native-vast-20261016`, windows-on-vast), a sloppy research loop toward ~300 fps (allowed only with explicit-failure gaps, each recorded), so that if PTX is chosen later
  the argument is: a sloppy kayfabe reaches ~300 fps; PTX gives that with guaranteed correctness and maintainability.
Candidates for the D3D11 gap, ranked [inferred]: Translated/CE-relay dependency (kernel channels pass through the worker), present/flip path, private-ring waits, GPU memory path (least likely).

## 7. Fallback ladder if P1 fails (not before the evidence says so)
(1) scan-for-life with a copy-engine batched gather and a cheaper parser (guess D3D11 140-170 fps); (2) PTX on the graphics engine for the whole Windows passthrough (best ceiling, most work;
probe [measured]: a kernel gather costs 612 us p50 and -10.5% fps at 5,000 launches/s, copy-engine copies 5 us and < 1% at 20,000/s, so launches must be batched to ~1,000/s with the latency hidden).
The binding-independent trigger rule (subchannel 5..7, method 0x200..0x203, data equal to a live handle) is judged fragile by the owner: an option of last resort, not planned.

## 8. Not doing now
PTX, reverse engineering, the inline vCPU copy path (phase 2) and the window CE gather (phase 3) before their gates, GP_PUT polling (never: the last doorbell covers every GP_PUT write before it),
scrapping scan-off on a case nobody has observed.

# V3 USERD relay — a Passthrough twin whose USERD the host cannot adopt

**STATUS: LIVE (implemented, measured), 2026-10-08 — behind `KF3_WIN_USER_CHANNELS_PASSTHROUGH` (default off).** `[measured, runs 68-72
at d67e9290..8de8ef26]` the Windows compositor's per-process channel is born over kayfabe's USERD, its doorbells are relayed (98 forwarded in
run71) and the engine consumes every entry (`host GP_GET = GP_PUT`); the guest's `GP_GET` lags by the documented staleness (3 entries at
release). Two fixes the first runs needed are folded in: the worker must treat a relayed twin as alive (`ChanPlane::alive`, run67), and a
Windows process space maps below host RM's default VA start (run69/70: `KF3_TWIN_VA_BASE`, diagnostic — ⊘ replaced 2026-10-08 by the
rule `kf_host::channel::MirrorVaStart`, measured in runs 73 and 76 at 3f23995a/f649d2c3). Record:
`traces/windows_code43_walls_20261007/README.md`, runs 67-72.
⊘ The original status line: *DESIGN-ONLY → being implemented behind the flag, 2026-10-08.*
Coordinator decision of 2026-10-08 (option (i) of `traces/windows_code43_walls_20261007/README.md`, "Stop (fourth session)"); option (ii),
a host IOMMU identity domain, is out (it touches the owner's live desktop GPU). Owner ruling context: `docs/OWNER_RULINGS.md` §V
("a translated can become passthrough, if you know at channel creation").

## 1. The problem (run61 at 883f878e, 2026-10-08)

`[measured, run61 at 883f878e, 2026-10-08]` A Windows per-process (user-work) channel classified Passthrough
(`kf_rm::chanlink::windows_user_work`) declares its USERD as a 512-byte slot of the guest RM's USERD pool in guest SYSTEM memory. A
Passthrough twin adopts the guest's USERD (`kf_chan::passthrough`: "ADOPT AT CREATION"), here through kayfabe's guest-RAM OS descriptor; the
host IOMMU (`DMA-FQ`) maps that page at IOVA `0x7ff9_f969_b000`, and host RM refuses the channel:
`kchannelCreateUserdMemDesc_GV100: physical addr size ... is incorrect!` → `NV_ERR_INVALID_ADDRESS` (`ogkm-580:
kernel_channel_gv100.c:211-219`). Linux guests never hit it: their user channels' USERD is in video memory.

## 2. The relay

The twin is born exactly as a Passthrough twin — over the guest's own GPFIFO VA, in the mirror of the guest's VA space, on an
asserted-USER host channel, push buffers never read — **except its USERD**: a 4 KiB **kayfabe-owned video-memory object**
(`HostRm::alloc_device_local`), not mapped in any GPU VA space (PBDMA reaches USERD through the instance block), with one CPU view kept by
kayfabe. Two 4-byte cursors are relayed between the guest's slot and this USERD; nothing else is copied.

### 2.1 The doorbell path (guest `GP_PUT` → twin)

- **The trap stays the Translated trap**: the twin's token is allocated with `Route::Translated`, so a guest doorbell write publishes the
  token's bit and wakes a worker (one CAS, no work, no lock, no ioctl on the vCPU) — `kf_trap::TrapPath::doorbell`, unchanged. The doorbell
  token translation is the plane's usual one (guest token = the guest's chid slot → host token of the twin).
- **The worker** (`ChanPlane::serve`, which already serves Translated tokens) finds the relay for that host token and runs one step:
  1. a plain 4-byte load of `GP_PUT` from the guest's USERD slot (guest RAM, the existing `UserdView::Ram`);
  2. **bound it**: a value `>= gpFifoEntries` (the entry count the guest declared at alloc, fixed for the twin's life) is not stored — the step
     is refused, counted, and the first one logged by name (the twin keeps its last good `GP_PUT`; the guest sees no progress: fail closed);
  3. if it differs from the twin's `GP_PUT`: a release fence, a plain 4-byte store of it into the twin's USERD through kayfabe's CPU view, a
     release fence, then the host doorbell (`HostRm::doorbell(token)`, the call Translated rings already make);
  4. the GP_GET write-back below.
- Chosen: the worker, not the trap — the copy is cheap, but the host doorbell from the trap would be a second inline host write the owner's
  ruling sanctions only for an adopted ring, and the worker already owns the token's BUSY state (no two steps of one twin run at once).

### 2.2 The GP_GET write-back (twin → guest)

⊘ *Correction (2026-10-08, runs 76-78 at f649d2c3/492fb3f0/42b332b3, above the text it corrects):* the per-doorbell write-back left
the guest's `GP_GET` at 0 on a twin rung once while the engine had consumed every entry (run76). The follow-up named below is built:
`kf_chan::userd_relay::refresh` (behind `KF3_RELAY_GET_REFRESH`, default off) writes the engine's `GP_GET` into the guest's slot on
every host non-stall wake (before the guest's interrupt is raised) and on the worker's park tick (`kf_chan::worker::TICK_TAG`), bounded
to `[0, entries)`, never reading `GP_PUT`, never ringing. The step now takes the relay's lock BLOCKING: with a `try_lock`, a refresh
holding the lock made a step return "served" and the doorbell was lost (run77). `[measured, run78 at 42b332b3, 2026-10-08]` guest and
host cursors agree at release; the guest's TDR was not caused by staleness (it is the refused async preempt,
`traces/windows_code43_walls_20261007/README.md`, sixth session).

- Who: the same worker step, after the doorbell (2.1, step 4): a plain load of the engine-written `GP_GET` from the twin's USERD, a plain
  store into the guest's slot. Host-derived only; the guest's value is never read back into the twin.
- When: at every doorbell of that channel. Between two doorbells the guest's `GP_GET` is stale by at most the work since the last one.
- What staleness costs: only delay. `GP_GET` tells the guest's kernel driver which ring entries it may reuse; a stale value makes the ring
  look fuller than it is, so the driver waits or rings again (which refreshes it). Completion itself is signalled by the work's own semaphore
  releases and fences, which the engine writes into guest memory directly (unchanged from Passthrough). `[inferred]` Windows' 32768-entry
  rings make a full ring from staleness alone unlikely; if a run shows a driver waiting on `GP_GET`, a refresh on the host non-stall
  interrupt is the follow-up.

### 2.3 Hostile input

- `GP_PUT` is a guest-chosen 32-bit index: bounded to `[0, entries)` before it is stored (2.1.2). The engine reads GP entries only from the
  guest's own GPFIFO VA in the guest's own mirrored VA space (as for every Passthrough twin), so no `GP_PUT` value can make the twin read outside
  that space; an out-of-range one never reaches the twin at all.
- `GP_GET` written to the guest is the engine's value from kayfabe's own object.
- No push-buffer word and no GP entry is ever read or copied by kayfabe.
- The guest cannot reach the twin's USERD: it is a kayfabe object in no GPU VA space and no guest mapping; the guest's slot is only read.
- A guest ringing the doorbell in a loop costs one CAS per write once the bit is set (the Translated trap's property).

### 2.4 Lifetime

- **Free** of the channel (or its TSG, device, client): the twin is freed as every Passthrough twin; the relay is removed with it and its
  CPU view released and object freed, in that order, after the host channel is gone (no store can reach a released view: the worker's step
  holds the relay's lock, and removal takes it).
- **Guest reboot / GSP re-init**: every twin is retired through the same free path, so every relay goes with its twin.
  (`[measured, run57 at 40230e23 and run62 at 883f878e, 2026-10-08]` an in-process guest reboot is not yet a clean second boot for Windows — a P4.5 gap, recorded separately.)
- **Allocation failure**: if the USERD object or its CPU view cannot be made, the birth is **refused by name** (`NV_ERR_INSUFFICIENT_RESOURCES`,
  "USERD relay: ...") and nothing is left behind; the guest's channel allocation fails (fail closed), never a twin without a relay.

### 2.5 What it does not change

The classification (`windows_user_work`), the twin's privilege (asserted USER), its VA space (the guest's mirror), the scheduling (the
guest's own `GPFIFO_SCHEDULE`), the error notifier (the twin's RC record lands in the guest's notifier, P5c), and the rule that kayfabe never
reads a Passthrough push buffer. ⊘ *Correction (2026-10-08, runs 73-76 at 3f23995a..f649d2c3, above the text it corrects):* the D3D twins'
RC was not a missing binding on subchannels 0-4 (NVIDIA's dev_ram.ref: the mapping is fixed and SetObject is not required by any engine);
every D3D device channel binds `NV50_DEFERRED_API` to SOFTWARE subchannel 5 (`SET_OBJECT 0x5080`) and host RM, with no such object on
the twin, raises PBDMA `DEVICE` (Xid 32). The labelled experiment `KF3_WIN_TWIN_DEFAPI_OBJECT` (default off) authors one host object of
that class on the twin from the guest's own alloc, nothing registered: no Xid in run76. `traces/windows_code43_walls_20261007/README.md`,
runs 73-76. The original sentence: Subchannel bindings (run57's inference) are authored only if a run shows the first segment RC'd for want of
them — from the guest's own class allocations on that channel, never from guest bytes.

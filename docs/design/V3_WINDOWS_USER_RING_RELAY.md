# V3 Windows user-channel ring relay: private GPFIFO + private USERD, with a safe migration path

**STATUS: DESIGN-ONLY, 2026-10-12 (proposal from the owner/coordinator discussion; see §9 for the owner's later refinements, which supersede §4.3 where they differ; the rulings are PROPOSED, the Codex session records them as §AI/§AJ).**
Supersedes nothing yet. Builds on `V3_USERD_RELAY.md` (private USERD, measured runs 68-78) and answers the decision note
`V3_WINDOWS_USER_FIFO_MIGRATION_DECISION_20261011.md` (on branch `codex/windows-matrix-20261011`, options A/B/C). MEASURED = read from a log, capture or
source file; INFERRED = a reading, must be checked.

## 1. The problem

GravityMark 1.89 Vulkan passes under VFIO (183 s) and fails under kayfabe: host Xid 32 on a Windows per-process (user-work, Passthrough) twin channel.

MEASURED (Codex, runs 1361/1510/1540, `traces/windows_matrix_20261011/gravity-vulkan/`):
- the faulting channel is Passthrough/user-work; it is scheduled and forwards 4096+ submissions BEFORE its one `FIFO_UPDATE_CHANNEL_INFO` registration (VFIO also
  registers late, 9.2 s after scheduling), so a registration-time conversion before token/schedule does not fit;
- the submitted `0x200` trigger carries the registered handle; the bundle is a ring GROWTH: 32768 -> 65536 entries, new GPFIFO VA `0x42be000`, USERD offset `0x600`;
- refusing the registration (`NOT_SUPPORTED`, run 1540) is NOT a fallback: device lost, host Xid 32, TIMEOUT 605 s.
MEASURED (ogkm 595.84 source): the control carries `RMCTRL_FLAGS_PRIVILEGED` (admin user or kernel caller only, `control.h:199`) so an unprivileged host twin can never
execute it; it needs client-allocated USERD (`bClientAllocatedUserD`) and a GSP client; `deferred_api.c:100` stores the registrant's privilege level and the trigger runs
with it; no code in the open tree calls it (Linux never initiates one). INFERRED: it belongs to the Windows memory-manager family of deferred calls (TLB invalidate,
evict/promote ctx, ZCULL/PM/preemption binds).

Why Passthrough cannot serve it as built: the host twin's GPFIFO is the guest's own ring at the guest's VA; the guest grows the ring by writing a new one elsewhere and then freeing
the old; the GPU keeps fetching from the old base unless a privileged control re-points it.

## 2. Idea (owner)

Give Windows user-work twins a PRIVATE host GPFIFO (vidmem, stable size) as well as the private USERD they already have. The guest's ring becomes only the SOURCE kayfabe copies
8-byte entries from; the host channel never has to follow a guest ring move, so the migration is supported in software with no privileged host verb. A channel that needs
translation can switch relay mode in place, because the ring and USERD are ours and we decide what advances.

## 3. Scope (keyed on the guest's declared state, never an OS name or a flag; §AE, D4)

- Applies to every Windows user-work twin (`kf_rm::chanlink::windows_user_work`), whatever its USERD aperture (a vidmem-USERD Windows channel avoids the sysmem bug but can
  still register and trigger a growth).
- Linux guests: unchanged. Their per-process USERD is in video memory (true Passthrough adopt, `V3_USERD_RELAY.md` §1) and the open Linux stack never initiates a
  migration. A `FIFO_UPDATE_CHANNEL_INFO` registration on any other channel kind is refused by name and counted (clean mode).
- Kernel (Translated) channels: unchanged.

## 4. Design

### 4.1 Private USERD (exists)
4 KiB kayfabe-owned video-memory object, not mapped in any GPU VA (PBDMA reaches it through the instance block); two 4-byte cursors relayed (`V3_USERD_RELAY.md` §2).
`GP_GET` is written back lazily (stale = delay only; completion comes from the work's own semaphores/fences).

### 4.2 Private GPFIFO (new)
- The GPFIFO base is a GPU virtual address in the channel's space (INFERRED from `gpFifoOffset` in `alloc_channel.h` and the twin being "born over the guest's own GPFIFO VA"; confirm in
  `kernel_channel.c`). So the ring needs a VA, unlike USERD.
- NOT at the guest ring's VA: the guest frees the old ring at migration and may reuse the VA; a host ring there would collide at exactly the moment it matters.
- Place it in a HOST-RESERVED VA RANGE the guest cannot reach: the host owns the mirror space's page tables and kayfabe replicates guest leaves into it, so kayfabe refuses
  (fail closed, counted) any guest leaf in that range. Needs the host space to extend past the guest's declared VA size (UNVERIFIED; kf-mem already has host-held
  reservations, "HELD BY HOST holes").
- Size chosen by kayfabe once (>= the maximum in-flight window, not the guest ring size); backpressure when full (do not ring; `GP_GET` lazy).
- Entries are copied 8 bytes at a time from the guest's current ring; they still point at pushbuffers in guest VAs (guest leaves, unchanged), so no pushbuffer payload is copied.

### 4.3 Doorbell path (weak boundary kept)
The trap stays the existing Translated trap (token bit + wake, one CAS on the vCPU, nothing else). Two paths, one state machine per channel:
- INLINE (fast): on the doorbell, bounded work only: load guest `GP_PUT`, bound it `[0, entries)`, copy the delta if it is at most K entries (K small, a constant), release fence, store `GP_PUT`
  into the private USERD, ring the host doorbell. Keeps the doorbell's semantics ("everything before this ring is scheduled after it") and adds no ioctl. Requires an owner ruling (§6.2).
- WORKER (default today, and for any delta above K): the worker does the same steps off the vCPU. A guest-reachable unbounded copy on a vCPU is a security bug, so the inline path never exceeds K.
Entries are validated before they are stored: length cap, no privileged-pushbuffer flag, refused-and-counted otherwise.

### 4.4 Migration support
The trigger `0x200` is a method inside a pushbuffer, not a GPFIFO entry type, and the host GPU would execute it. Two variants; a measurement gate (§7 P0) chooses:
- **A-lite (no pushbuffer parsing):** accept the guest registration (real GPU answers OK) and author a harmless host registration for the SAME handle on the twin's real `5080` object,
  so the host GPU executes a no-op-equivalent at the trigger (candidate: `DMA_INVALIDATE_TLB` of the twin's own space; `deferred_api.c` special-cases it with `bIsCtrlCall = NV_FALSE`, so it may not hit
  the PRIVILEGED check: UNVERIFIED). The relay source switches to the new guest ring/USERD by a rule that must come from measured cursor semantics (which USERD gets `GP_PUT` after the trigger,
  whether indices continue). Only valid if that rule is exact.
- **PARSE mode:** when a `FIFO_UPDATE_CHANNEL_INFO` registration arrives, kayfabe HOLDS the registration reply, sets the channel to PARSE before replying, and replies. The guest can only use the handle after the
  reply, and a doorbell relayed after the flag parses every newly published pushbuffer segment (bounded; framing and object binding checked; never edits guest buffers in place; no
  CPU execution of GPU work), finds the trigger at its exact stream position, replaces it with a host-authored ordering/no-op in a kayfabe-owned COPY of the segment (staging memory also in the
  reserved range) and, at that position, switches the relay source to the validated new ring/USERD. Segments published before the flag cannot contain a valid trigger (the handle was
  unregistered). Race-free by construction because kayfabe owns the reply. Costs reading pushbuffers (`8E + 4P` bytes per submission, estimated, unmeasured) only on channels that registered.
PARSE stays on until `REMOVE_API` for that handle. Other deferred bundle kinds on these twins remain refused by name at registration.

### 4.5 Migration validation (hostile guest)
`hChannel` must be this channel's own handle; `hUserdMemory` must belong to the same client and be client-allocated USERD; `userdOffset`/`gpFifoOffset` aligned and in bounds; `gpFifoEntries` a power of two within a cap;
the new ring's complete span resolved through the guest page tables to backed guest pages; mapping/backing lifetime tracked (free/reuse races against the VA manager gate); cursor transfer defined from measured native
semantics; a malformed bundle is refused by name and counted, never panics; nothing blocks a vCPU or a lock a vCPU takes; no unbounded guest-reachable work.

## 5. What this does not do

No privileged host verb, no change to the guest's RM, no heuristics or timeouts that release a stuck state (§AE). The optional host-patch tier (H5 USERD address-size fix, H8 validated own-channel migration in
`V3_HOST_PATCH_LIST.md`) lets a patched host adopt the guest ring/USERD in place and let RM migrate; this design is the stock-host answer, and with the patch most of it disappears as intended.

## 6. Rulings needed from the owner (proposed wording)

1. **§AB.2 amendment (proposed):** "In a Passthrough space nothing kayfabe- or host-owned may be mapped, EXCEPT one host-owned GPFIFO ring mapping (and, in PARSE mode, its staging region) per relayed Windows
   user-work twin, placed in a VA range reserved for the host that the guest cannot reach; guest leaves in that range are refused and counted."
2. **Inline doorbell write:** allow the bounded inline path (§4.3) for relayed twins (the earlier decision allowed the worker only).
3. **Scope:** the private ring is a birth-time property, so it applies to all Windows user-work twins at birth (no per-channel indicator exists before the registration).

## 7. Plan and gates

- **P0 measurements, no hardware (Codex has the data):** (a) cursor continuity around the trigger: which USERD receives `GP_PUT`, `GP_GET/GP_PUT` in old/new USERD, entry indices; (b) ring occupancy at registration (run 1510);
  (c) apertures of the migrated objects; (d) whether the VFIO reference executes the update (a plain RmControl `0x20801116`) or only registers; (e) whether the host mirror space can exceed the guest's declared VA size; (f) whether the
  GPFIFO base is a VA (`kernel_channel.c`); (g) how the real RM treats an unregistered `0x200` handle and whether an unprivileged `DMA_INVALIDATE_TLB` registration is accepted. Gate: if (a) gives an exact rule and (g) holds, try A-lite;
  otherwise PARSE mode.
- **P1:** owner decides §6.
- **P2:** GPU-free failing-first model tests of the pure state machine: reserved-range refusal, entry bounds, inline-vs-worker bound K, held-reply mode switch ordering, ring wrap, backpressure, hostile bundles, free/reuse races.
- **P3:** implement behind a flag (default OFF, never required to boot), independent review (hostile-guest, non-blocking), Linux regression on the exact revision (gates 9/9 + gate 10, fast 30/30).
- **P4:** hardware: GravityMark Vulkan against the VFIO reference (PASS 183 s with output image), one variable per run, falsifier first; measure submit-to-completion p50/p99 and CPU versus the current relay. Success: PASS, no host Xid 32, no guest 153/4101 beyond baseline.
- **P5:** success: remove the flag and hardwire (flag rules); failure: write the evidence under "Decisions waiting" and fall back to the patch tier.

## 8. Unverified (each has a falsifier above)

GPFIFO base is a VA; the reserved range is possible; an unprivileged harmless registration is accepted and executed; native cursor semantics are exact enough for A-lite; Windows tolerates the switch; the guest never relies on the
old ring after the trigger. The cause of the growth is not established (ring growth is measured; why it is requested is not).

## 9. Revision 2026-10-12 (owner): the doorbell table gets a third mode; the vCPU never waits

Context: Codex's review (`V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md`) and ruling §AI (on `codex/windows-matrix-20261011`) approved GPU-read-only private command mappings with worker-only submission and
command inspection from birth, and rejected the inline path and the late switch. The owner's refinements below reverse those two points; the owner and Codex must reconcile §AI (record the result as a new
ruling in the Codex branch, which owns §AI; this repo's `OWNER_RULINGS.md` is not touched here to avoid a conflicting append).

**What is required at birth in every design (agreed):** a private USERD and a private GPFIFO ring, because the host channel's ring base is fixed at birth, the privileged control cannot re-point it on a stock host, and the guest frees its
old ring after a migration. The entry relay copies 8-byte entries only (about `8E` bytes per submission).

**The doorbell table has three modes per matching token** (the table runs inline on the vCPU or from ioeventfd, equivalent):
1. *Passthrough:* ring the host token inline on the vCPU (unchanged; Linux, kernel-visible simple channels).
2. *Translated / emulated:* wake the drainer and a worker (unchanged).
3. *Migratable Passthrough (new):* inline on the vCPU: check the guest `GP_PUT`; if the delta is at most K entries, copy the entries into the private ring, store `GP_PUT` into the private USERD behind a release fence, ring the
   translated host doorbell, no syscall, return; if the delta is larger, or anything below fails, wake a worker as in mode 2. Pushbuffers are NOT inspected in this mode; the entry pointers are copied as-is.
At birth a Passthrough channel gets a flag "migration possible later" (true for Windows user-work and for sysmem USERD; declared state, never an OS name or an environment flag). With the flag it NEVER enters mode 1; it enters mode 3.
Late switch: when the guest registers the `FIFO_UPDATE_CHANNEL_INFO` bundle, the table atomically flips the channel from mode 3 to a restricted Translated mode (§4.4 PARSE), set BEFORE the registration reply is released, so
a cooperative guest's trigger (written after the reply, in entries published after it) is parsed. The restricted mode is the GPU-read-only command-copy variant, not the privileged T-space.

**The vCPU never waits (owner rule):**
- Per-channel state word: `IDLE`, `INLINE_BUSY`, `WORKER_BUSY`, plus a `PENDING` bit. The vCPU does ONE compare-and-swap `IDLE -> INLINE_BUSY` (the channel's existing BUSY flag). On failure it sets `PENDING`, wakes a worker and returns:
  no spin, no lock wait, no syscall, no allocation inside the inline critical section (so no lock a vCPU takes is ever held across a block).
- Whoever releases (inline owner or worker) swaps the state to `IDLE` and re-checks `PENDING`; a set bit means one more step by a worker. Lost wakeups are impossible: `PENDING` is published before the failed vCPU's wake, and the releaser re-checks after the release.
- Every step computes its delta from the CURRENT private cursor to the CURRENT guest `GP_PUT`, so racing doorbells coalesce, nothing is copied twice or skipped, and the order of handoffs does not matter. The single-writer rule covers the private ring and USERD.
- The mode flip is made by the act thread by CAS-ing the state to a `SWITCHING` value; a vCPU that fails its CAS takes the failure path (pending + wake) and sees the new mode on the next doorbell.
- Test it with an exhaustive interleaving model (loom or a state-machine simulation), failing first: two vCPUs racing, a worker racing a vCPU, a switch racing a doorbell, wrap, a full private ring (room check fails: wake the worker, never block, never overwrite).

**Further guards:** the ring needs a guest-cursor to host-cursor pairing after the guest grows its ring (indices no longer match; the lazy `GP_GET` write-back maps back through it); entries are validated before they are stored (length cap, no privileged-pushbuffer
flag); if the guest ring is in guest-visible video memory, each entry read from a vCPU is a PCIe read, so K is small and measured (P0c gives the aperture); an unparsed trigger on an unprivileged twin executes nothing and only RCs the guest's own channel
(measured: Xid 32, run 1540), the exposure every Passthrough channel already has from a malformed method; the two-VM check (an RC in one VM while another runs a workload) confirms it stays contained and is a gate before the late switch is trusted.

**Decision for the owner and Codex:** inline Migratable Passthrough plus late switch (this section) versus Codex's worker-only mediation from birth (§AI as written). The tie-breaker is the measured submit-to-completion latency and CPU cost of snapshots on the apps that pass today (Edge video, GravityMark D3D12, the desktop).

**Correctness versus optimisation (owner, 2026-10-12):** the worker path is the correctness baseline; the inline path of mode 3 is an optimisation. Build and validate the worker path (private ring and USERD, entry relay, mode flip, migration) first; add the inline fast path only if the measured submit latency shows the worker wake matters.
Note: with `ioeventfd` the kernel only signals an eventfd, so the inline copy cannot run there; an ioeventfd doorbell always takes the worker path. The inline mode exists only on the userspace trap path on the vCPU thread, so the two doorbell mechanisms are equivalent for the wake, not for the inline copy.

**Linux guests and the guest doorbell helper (`V3_GUEST_DOORBELL_MODULE.md`):** the optional helper lets a Linux guest write the real host doorbell page for passthrough tokens without a VM exit. A token whose channel carries the "migration possible" flag must never be eligible for that guest-direct route (kayfabe must see its doorbell to copy the entries). With the flag set only for declared Windows user-work (the helper is a Linux-guest module and Windows has none), the module needs no change; keep the invariant and pin it with a test: flagged tokens are always routed through kayfabe, whatever the guest helper's table says.

# V3 Windows user-channel ring relay: private GPFIFO + private USERD, with a safe migration path

**STATUS: DESIGN-ONLY, 2026-10-12 (proposal from the owner/coordinator discussion; the §AB.2 amendment in §6 is PROPOSED, not yet a ruling).**
**Correction, 2026-10-24 (family scope):** the private ring proves progress with two words
the ENGINE writes back into its private USERD: `GP_GET` (fetch credit) and `REFERENCE` (the
marker's `SET_REFERENCE`, the gate proof). Both are C56F contracts. Blackwell's control
struct names only `GPPut` (`clc96f.h`, `clca6f.h`), and `kf_chip::Family::engine_writes_userd_gp_get`
is false there. Public RM never pushes `SET_REFERENCE` on Hopper or later
(`ogkm-595.84 kernel_rc_watchdog.c:1416-1446`). On GB205 round B1n (lease 57472433,
binary cb80cbd29, which admitted the generated non-Ada classes), both relayed DWM rings
took Xid 32 METHOD. INFERRED, not measured: this was at the relay's first publication, since the
trace's first turn and the Xid fall in the same 100 ms. The relay-off arm of the same lease, with
the same USER twin privilege, held 10 minutes with no Xid. The route is now one decision with
the birth admission (`userrelay::ring_relay_route`), so such a family takes the USERD-relay
route. A Blackwell-capable private ring needs another progress proof, which is an owner decision.
**Correction, 2026-10-13:** a prepared native deferred relocation at source
`1a666d783` succeeds on RTX 4070 / 595.91.07 as UID 65534 / CapEff zero, with
replacement-only GPU marker/USERD GET and matched NOP controls. Thus §1's
"an unprivileged host twin can never execute it" is falsified for the deferred
V2 path. Direct-control annotations and earlier same-config Xids do not prove
that impossibility. Cursor, relay, lifetime and per-driver gates remain open;
this is not a Windows fix. See [native assessment](V3_NATIVE_FIFO_RELOCATION_20261013.md).

**SUBSEQUENT OWNER CLARIFICATION, 2026-10-12:** native USER privilege and own
explicitly admitted/refreshed mappings are the access boundary. Ordinary GP
references may remain unchanged after scan-from-first-submission; stage only
intercepted trigger segments. A mutable ordinary pushbuffer alone does not
justify mandatory full snapshots. CPU scanning never authorizes a host action
or proves GPU execution/completion. This supersedes blanket immutable-copy
requirements below; exact ownership, stream order and containment still need
validation. Linux Passthrough still forbids host-owned GPU mappings. See §AI.

**LATEST OWNER/ARCHITECTURE UPDATE, 2026-10-12:** the owner's subsequent
three-mode clarification in OWNER_RULINGS §AI supersedes the blanket worker-only
and inspection-from-birth wording immediately below. Direct native-token dispatch,
bounded pre-mapped USERD/GP-descriptor copy (large delta off-vCPU), and worker-only
inspection are the intended policies. No pushbuffer inspection on a vCPU. Linux
Passthrough still forbids host-owned GPU mappings. The first atomic prototype is
unconnected; neither inline runtime submission nor GravityMark PASS is claimed.

Conditional conversion still needs two distinct proofs: exclude old CPU
publishers, then preserve any existing native parser/binding/ordering state.
A never-published channel can avoid the second problem. Run1361's deferred-API
object allocation precedes its first recorded submission, unlike registration;
selecting worker mode at object-allocation admission is therefore a candidate.
A previously published opaque stream remains gated. The public FIFO manual
allows hardware method packets across GP boundaries, so an appended witness and
empty GET/PUT cannot establish a parser boundary. See the
[architecture review](V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md)
for rationale, evidence, model scope and remaining implementation gates.

**REVIEW UPDATE, 2026-10-12:** imported from integration commit328fe4bf8.
The owner has now approved a narrower exception: GPU-read-only private rings and
command staging for declared Windows user-work, worker-only submission, command
inspection from birth; **Linux Passthrough still forbids host-owned mappings**.
See OWNER_RULINGS §AI and [the P0 review](V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md).
This supersedes the pending wording above and §6 only to that extent. No inline
copy or writable private fence is approved. No VM implementation or GravityMark
PASS is claimed. The registration-reply race-free claim below is rejected;
inspection must start at birth. Address reservation prevents collisions, not GPU
access; actual read-only PTEs must protect integrity. The plain-RPC criterion in
P0(d) cannot establish deferred execution inside GSP. Native cursor continuity,
safe staging retirement and migration ordering remain measurement gates for both
variants; failure to prove A-lite does not automatically establish PARSE safety.

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

## 9. Coordinator revision 2026-10-12 (after the owner's three-mode refinement and Codex's architecture review)

This section is appended to Codex's current copy of this document (taken from `codex/windows-matrix-20261011` at `ea8b77ee1`) so both lines carry the same text. Dispatch mechanics are in
`V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md`; this section records only what is new or corrected.

**Corrected (coordinator):** my earlier "late switch at registration, race-free because kayfabe holds the reply" is withdrawn. Two reasons: (1) a hostile guest can publish the handle before the reply and an already-forwarded mutable
pushbuffer can still be fetched later; (2) stronger, and valid for a cooperative guest too: starting inspection mid-stream needs parser state (subchannel binding, hardware method packets that span GP segments, persistent HOST methods,
pending MEM_OPs) that an opaque history does not give (public FIFO manual cited in the architecture review). The sound variant is the owner's earlier idea: convert BEFORE the channel has published any native work, i.e. at the
`5080` object-allocation admission (run 1361, channel 4e: birth 8311, 5080 alloc 8315, schedule 8365, first forwarded submission 8437), with old-claim exclusion before the object is acknowledged and the channel's "never published native work" history bit checked.

**Open value question (measure):** the earlier measurement says the `5080` is the first and only `ENG_SW` child of EVERY Windows channel. If every Windows user-work channel allocates it before its first submission, conversion at object allocation
is in effect inspection from birth for all of them, and the CopyRing mode (inline or worker) rarely applies on Windows. Count, over the matrix runs, the Windows user-work channels that allocate a `5080` before their first forward, and those that do not.

**Candidate to evaluate (cheaper inspection):** scan the pushbuffers from the first submission WITHOUT copying them (read-only scan for a `0x200` on the subchannel bound to the `5080` object, tracking packet/binding state from birth), forward the original GP entries unchanged,
and take an immutable snapshot only for a segment that actually contains a trigger (rare). Safety rests on hardware containment, not on the scan: the twin is an unprivileged USER channel in its own restricted space with the physical-operand prohibition, and an unscanned trigger on it executes
nothing and RCs only the guest's own channel (measured: Xid 32, run 1540; the exposure every Passthrough channel already has from a malformed method). Gates: the two-VM check (an RC in one VM while another runs a workload stays contained); Codex's concrete counterexample, if any, of a capability an unscanned trigger gives an unprivileged guest;
a measured cost against full snapshots. If it holds, the private read-only command mappings and stores are used only on the trigger segment, which removes most of the always-on machinery.

**Kept from the owner's discussion:**
- The inline (vCPU) CopyRing path is an optimisation; the worker path is the correctness baseline and is built and validated first. With `ioeventfd` the kernel only signals an eventfd, so the inline copy cannot run there; an ioeventfd doorbell always takes the worker path.
- The vCPU never waits: one compare-and-swap (the channel's BUSY word); on failure it sets a durable PENDING bit, requests a nonblocking worker wake and returns; the releaser re-checks PENDING; every step computes its delta from the current private cursor to the current guest `GP_PUT`, so doorbells coalesce (modelled, native and Loom atomics, in `kf-trap`).
- Linux guests and the guest doorbell helper (`V3_GUEST_DOORBELL_MODULE.md`): the helper already writes the real host doorbell page only for a table route `Passthrough` and rings kayfabe's classic trapped page for `Emulated`, unknown, not found or a missing table ("any doubt rings the original kayfabe page"). Publish a flagged (CopyRing/Worker) token with a route that is not `Passthrough` and the module needs no change;
  pin it with a test: an unknown route value takes the classic path.

**Correction 2026-10-13 (owner), withdrawing my "background scan, enforce later" idea:** a scanner that trails behind the GPU has no safe boundary. Kayfabe does not forge completions, so once the GPU completes a submission the guest may reclaim and
overwrite its pushbuffer; a lagging scanner would read recycled memory and its parser state would be garbage. Any scan (or snapshot) must therefore happen BEFORE the entry is forwarded, on the submit path. What stays valid: a synchronous READ-ONLY scan
without copying (Translated already copies almost nothing: run 1660's kernel copy channel 0x80c had 35841 submissions, 5887 splits, 1 store view), with a snapshot only for a segment that contains a trigger. Scan cost is microseconds per submission; the measured gap
(kayfabe Vulkan 103 FPS, median frame 9.41 ms, versus 711 FPS, 1.29 ms under VFIO; `traces/windows_default_route_20261013/PERF_GRAVITYMARK_VULKAN.md` on `claude/default-route-evidence-20261013`) is milliseconds per frame, so scanning is unlikely to be the dominant cost.
Lead hypothesis to test first (INFERRED): the guest `GP_GET` write-back is lazy (refreshed on a worker park tick or a host non-stall wake, `V3_USERD_RELAY.md` §2.2), and a guest that waits on `GP_GET` for ring progress or fences is paced by that latency. Falsifier: refreshing `GP_GET` on every worker wake and every completion
interrupt leaves FPS and the median frame unchanged.

**Owner decision 2026-10-13 (proposed ruling text for the Codex session to record): pushbuffer integrity is not a security boundary for USER channels.** The scan and the GPU both run under the same contained privilege, the channel's own (unprivileged USER channel, own restricted VA space, physical-operand prohibition). A guest that changes a pushbuffer between kayfabe's read and the GPU's fetch,
or that makes the GPU fault on a bad VA, can only harm its own channel: a VA fault or RC on an unprivileged channel is benign for host and cross-process memory safety. So the immutable-snapshot requirement for ORDINARY submissions is dropped: forward the guest's GP entries (the pointers) unchanged; do a synchronous READ-ONLY scan before forwarding only to find the `0x200` trigger on
the `5080` subchannel (parser state tracked from birth); take a kayfabe-owned copy only of a segment that contains a trigger (the replacement), where a race with the guest still affects only that guest. What this does NOT remove: (1) the trigger itself must be found before the entry is forwarded (blind pointer copy alone fixes the private ring but still lets the unscanned trigger RC the channel);
(2) the private GPFIFO and the rare staging region remain the only host-owned mappings in a USER space (§AI); (3) availability, not memory safety: the blast radius of a channel RC on other VMs is still unmeasured (`V3_CHANNEL_RC_SCOPE_20261012.md`, M1-M3); a malicious channel can already cause one today with a malformed method.

## 10. The owner/Codex lifecycle plan, 2026-10-13, and what must hold for it

> ⊘ **UPDATE 2026-10-16:** implemented (PHASE 1, worker-only, GPU-free) as Scan / Pointer / Window in
> [`V3_USER_RING_LIFECYCLE_20261016.md`](V3_USER_RING_LIFECYCLE_20261016.md). Read it for the states, the flip protocol (a wait-free registry epoch instead of a held
> reply, until an inline claimant exists), the invariants and tests. Corrections to the preconditions below, above the text they correct: **P2 is contradicted** as a general
> property (the driver splits inline uploads across GP entries; 55 of 196 recorded refusal-trace segments end inside an open packet, Minecraft has 27 334 SYNC_WAIT entries behind
> open packets in a run), so a Window that starts with fresh parser state is a measured probability (`straddles_window`, `window_fresh_start_parse_error`), not a guarantee;
> every registration kind (not only `FIFO_UPDATE_CHANNEL_INFO`) opens a Window, because the relay executes the other bundles at their triggers too.

Plan: (1) a Windows USER channel is scanned from birth only until its `5080` SET_OBJECT has been seen (the subchannel binding is then known); (2) it then becomes *migratable passthrough*: GP entries copied as pointers, pushbuffers not inspected (inline for small deltas, worker otherwise);
(3) when the guest registers the `FIFO_UPDATE_CHANNEL_INFO` bundle (the privileged inner control reaching kayfabe's GSP emulation) kayfabe holds the reply, flips the channel to the scanning (Translated-like) worker mode race-free (old-claim exclusion, modelled in `kf-trap`), and keeps scanning until it reads the trigger;
(4) it applies the source switch at the trigger position and returns to migratable passthrough; a later registration (ring growth repeats: 32768 to 65536 to 131072) starts the cycle again. It avoids scanning every submission of every channel for life, and fits the owner rulings (pushbuffer integrity is not a security boundary; the vCPU reads only bounded ring entries; scan off vCPU before forwarding).
It is a good plan. These are the preconditions, each checkable with a shadow-mode run over the matrix and the corpus:
- **P1: the `5080` SET_OBJECT occurs once at setup.** Not measured. Count, per channel, the SET_OBJECT methods for the `5080` and the subchannel they use. Rule if it never appears: the channel keeps scanning (deterministic, no timer or submission-count threshold, §AE).
- **P2: packet boundaries at the switch (the real gap).** Knowing the subchannel does not give parser state: the hardware lets a method header and its arguments span GP segments (public FIFO manual, cited in the architecture review), so after unscanned entries the scanner may start mid-packet. Either measure that the Windows driver always ends GP segments on packet boundaries (shadow mode over all matrix runs; a violation is a counterexample), or find the trigger with a state-free signature:
  the registered handle value (the data word) preceded by the `0x200` header for the bound subchannel (the captured trigger is a count-1 INC method; compressed/NON_INC forms must be enumerated). A cooperative guest is covered; a hostile one can only RC its own channel.
- **P3: the registration reply is held while the mode flips**, so a trigger written after the reply is in an entry published after the flip (entries published before cannot hold a valid trigger for a cooperative guest). Concurrent submitters on the same channel are covered by the claim/exclusion protocol (CAS plus durable pending bit).
- **P4: what keeps a channel scanning.** Convert only for a `FIFO_UPDATE_CHANNEL_INFO` registration (inner command), not for other bundle kinds, and leave scanning on `REMOVE_API` or on use; a bundle registered and never triggered keeps the channel in the slower mode (a cost, not a failure). Count how often that happens.
- **P5: cursor and `GP_GET` semantics at the source switch** (measurement gate, unchanged): which USERD receives `GP_PUT` after the trigger, index continuity, and the guest-visible `GP_GET`.

**P2 candidate answer (owner idea plus the captured evidence), 2026-10-13: scan only SHORT GP entries.** The captured trigger (run 1510, `traces/windows_matrix_20261011/gravity-vulkan/late-fifo-89e8ddf8/README.md`, `GP[0x4b63]`) is its own four-word segment `8000c01e 8000c009 2001c080 4000002c`
(two immediate-data methods on subchannel 6, then INC method `0x200` count 1 on subchannel 6, data = the registered handle `0x4000002c`; hand-decoded from the header bits). A GP entry's length is in the entry itself, so deciding whether to look needs no pushbuffer read. Rule to evaluate: read-only scan only the entries with length at most L dwords (L about 8, at most 32 bytes),
looking for a `0x200` header followed by a registered handle (state-free; no parser state needed because the entry holds whole packets); skip every long render segment. Unprivileged USER channels have no physical operands and guest VRAM is not mapped into them, so no operand scan is needed at all. It relies on how the Windows driver emits the trigger (a convention, not a hardware guarantee): verify on the second growth of run 1630 and across the
matrix in shadow mode (entry length, subchannel and header position of every trigger; also the aperture of each scanned pushbuffer, sysmem versus vidmem). A cooperative guest is covered; a hostile one only RCs its own channel. This makes scan-from-birth cheap enough that the setup-only window and the registration switch of §10 may not be needed.

**Scope decision (owner, 2026-10-13): migratable passthrough is for Windows user-work channels with sysmem USERD only; vidmem USERD and vidmem pushbuffers are not supported on this route.** Basis: Linux guests' per-process USERD is in video memory and the open Linux stack never migrates (`V3_USERD_RELAY.md` §1); Windows' is in guest system memory (measured run 61). Written as an explicit scope with clean fallbacks, not as an assumption the code relies on
(OWNER_RULINGS §AI: allocation requests and declared OS identity are guest-controlled; checks must stay safe if another OS declares the identity):
- a user-work channel whose USERD is NOT in guest system memory is not relayed: it stays ordinary Passthrough (the Linux behaviour), its `5080` is the inert guest-graph node, and a `FIFO_UPDATE_CHANNEL_INFO` registration on it is refused by name and counted;
- a scanned GP entry whose pushbuffer is not in guest RAM (GPA) is forwarded unscanned and counted (`unscannable_entries`); a trigger inside one RCs only that guest's own channel; the counter shows whether this ever happens;
- NOT measured: the aperture of Windows pushbuffers (the shadow run logs it per scanned entry). Windows' use of vidmem USERD is "unlikely, possible" per the owner; the first user-work channel seen with vidmem USERD under Windows is a finding to record, not a crash.

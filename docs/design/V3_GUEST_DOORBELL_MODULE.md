# V3 — optional guest doorbell helper module

**STATUS: DESIGN-ONLY, 2026-09-26.** Owner idea; security posture approved 2026-09-26 (guest root
writing the real host doorbell page — able to ring any host token — is accepted risk, the same
posture as shared-GPU CUDA containers on vast/runpod). Nothing is built. Stock guests without the
module keep today's trapped path unchanged.

## 1. Why

Measured (kf3 fat guest, Qwen2-0.5B eager decode, nested vast boxes): ~1,008 doorbells per token,
each a trapped MMIO write = one VM exit (~50 µs nested; in-kernel ioeventfd floor ~20 µs; published
nested DevNotify ≈ 22 µs, non-nested ≈ 2.3 µs — ASPLOS'20 DVH, Table 3). Doorbells are ~99.7 % of
trapped exits and 55–80 % of the LLM's guest-vs-host gap. A write fault handled INSIDE the guest is
a guest `#PF`, not a VM exit (~1–3 µs even nested), so moving the per-doorbell decision into the
guest removes the exit for passthrough channels.

## 2. The design (owner's trap-and-route)

kayfabe exposes to the guest, both at setup (memslots are setup-only):
1. **The real host doorbell page**, read/write, as its own memslot (a second view beside today's
   usermode page, which stays write-trapped).
2. **The doorbell table**, read-only shared memory: `guest token → (host token, route)`, where route
   is `Passthrough` or `Emulated` (Translated/kernel channels, or anything kayfabe must see). Guest
   visible and not secret. ★ Owner (2026-09-26): each entry is ONE u64 word read with a single atomic
   load — the same atomic-integer word discipline as kayfabe's existing token words — so no seqlock is
   needed: indexed by guest token, the word packs `host_token | route | valid`.

The module, loaded optionally in the guest:
- When the stock driver maps the usermode/doorbell page into a userspace process (libcuda), it
  substitutes its own page: reads pass through to the host page (the page also carries the
  microsecond timer); writes are made to fault in the guest.
- On a write fault it decodes the stored token, looks it up in the table and:
  - `Passthrough` → writes the **host** token into the real host doorbell page (no exit);
  - `Emulated`, unknown, not found, or the table unavailable → writes the guest token into kayfabe's
    classic trapped page, so kayfabe handles it exactly as today. ★ The module is ALWAYS
    OPPORTUNISTIC (owner, 2026-09-26): any doubt rings the original kayfabe page.
- Coalescing is free: a doorbell only makes the GPU re-read the channel's `GP_PUT` from USERD (the
  twin is born over the guest's USERD), so the module may skip rings while one is outstanding.

### Discovery and inertness (owner, 2026-09-26)

The module talks to kayfabe over a small **virtio command set** (discovery, the table's location and
version, the host doorbell page's location, counters). It is **inert** — installs no hook, touches
nothing — if the virtio device does not initialise or kayfabe is not detected, and it **reports**
that it is inert. ⇒ Harmless on bare metal and under any other VMM. A **Windows** version is an
end-stage goal (feasibility to be established then).

### Lifecycle and the BAR1 doorbell (owner, 2026-09-26)

**Guest-visible resources** (exposed by kayfabe, discovered over virtio): a BAR holding (a) the host
doorbell TABLE — mapped **write-back, read-only**, so the module's per-doorbell lookups hit the CPU
cache — and (b) the **hardware host doorbell page**, read/write.

**BAR1 doorbell mappings are racy, and the two directions are asymmetric:**
- **Map (may defer the optimisation, never the mapping).** When the guest maps a BAR1 usermode
  doorbell view, kayfabe immediately places its OWN trapping doorbell page at that BAR1 location — the
  map succeeds at once and every doorbell through it is handled by kayfabe's trap, correctly. It then
  queues "BAR1 doorbell mapped at X" to the module over the virtqueue; the module may switch that
  mapping to the fast path at any later moment. Until it does, nothing is lost: doorbells that did not
  go through the fast route go through kayfabe's handler anyway.
- **Unmap, or another page placed at that address (may NOT defer).** kayfabe must not complete it
  while the module could still route writes for that mapping: it waits until the guest module has
  removed its interception of that mapping (acknowledged over virtio). Only while the module is
  loaded (the flag below).
- All other BAR1 edits are unchanged.

**Load/unload — free, unordered, independent of nvidia.ko** (either may be loaded first, either may
go away first):
- **On load:** the module announces itself over virtio. kayfabe, under the lock that orders BAR1
  doorbell changes, sets the *module-present* flag FIRST — so no doorbell unmap can slip between the
  check and the set — and then **replays** every interception area that already exists (current
  doorbell mappings), so the module can take over mappings made before it loaded.
- **Before unload:** the module restores every interception area to the original mapping, tells
  kayfabe over virtio (kayfabe clears the flag), and only then unloads.
- **Inert case** (bare metal, or virtio not initialised, or kayfabe not detected): the module never
  hooks anything, so unload is trivial.

⚠ **Design risk to close before building — the unmap wait.** kayfabe's unmap is driven by the guest's
own BAR1 page-table change and invalidate; if the module's acknowledgement needed a vCPU that is busy
inside that same RM path (a 1-vCPU guest, RM spinning with preemption disabled), the wait would never
end. Mitigation: the module drops its interception synchronously in the VMA teardown path (the
userspace mapping is torn down before RM frees the BAR1 mapping and invalidates), so in practice the
acknowledgement is already recorded when kayfabe looks; kayfabe's wait itself runs on a device
thread, never a vCPU, and must have a named, logged timeout outcome rather than an unbounded spin.
To be verified against ogkm's unmap ordering before implementation.

### BAR0 doorbell page, and multiple GPUs (owner, 2026-09-26)

- **BAR0 doorbell page — static, the easy case.** It never moves, so it has none of BAR1's map/unmap
  race: only userspace mappings of it come and go (per process), handled by the same VMA hook; no
  unmap wait is needed for BAR0. **kayfabe tells the module where it is** rather than the module
  hard-coding it: the offset/size is per architecture (GA10x `NV_VIRTUAL_FUNCTION` usermode page at
  `0xBB0000`; Hopper/Blackwell differ), and kayfabe already derives it per family
  (`kf-trap` memmap/trappolicy). The virtio discovery reply carries, per GPU: BAR0 doorbell
  offset+size, current BAR1 doorbell mappings, the table BAR, and the host doorbell page.
- **Multiple GPUs — required on both sides.** kayfabe: one kf3 device per host GPU, each with its own
  host RM session, host doorbell page, doorbell table and table BAR. Module: per-device state keyed
  by the guest PCI function (BDF); it attributes a doorbell mapping to its GPU by the BAR physical
  address the VMA maps; tokens are per GPU, so tables are per GPU. Virtio: one virtio function per
  kf3 device (preferred — each GPU independent) or one control device with a device id in every
  message.
- ⚠ Multi-GPU kf3 itself is **untested**: every measured run used one `-device kf3-gpu`. Two devices
  in one VM (distinct host GPUs, and later the same GPU twice) is its own test before the module.

## 3. Findings from ogkm-580 (the guest driver)

- **The token is guest-computed; a table is required.** For user channels the guest RM builds the
  work-submit token locally from the guest chid — `kchannelCtrlCmdGpfifoGetWorkSubmitToken_IMPL`
  (`kernel_channel.c:3283-3349`), `kfifoGenerateWorkSubmitTokenHal_GA100`
  (`arch/ampere/kernel_fifo_ga100.c:178-240`); the control (flags `0x10008`) never reaches the GSP,
  so kayfabe cannot answer it with the host token (also recorded at
  `crates/kayfabe-chips/src/ga10x.rs:486-495`). It is also copied into the channel's notifier
  (`kchannelNotifyWorkSubmitToken_IMPL`, `kernel_channel.c:4077-4093`). ⇒ the "answer the host token
  and skip the fault" shortcut is NOT viable; the table lookup is.
- **Token encodings differ per family**: one generator (`_GA100`) serves GA10x/AD10x/GH100; Turing,
  GB20x and GB100 each have their own (`generated/g_kernel_fifo_nvoc.c:636-656`). The table stores
  full tokens, so the module is encoding-agnostic.
- **Kernel/Translated channels ring via RM's own register mapping**: `kfifoRingChannelDoorBell_GA100`
  (`:966-995`) → `kfifoUpdateUsermodeDoorbell_GA100` (`:153-165`) → `GPU_VREG_WR32`; GH100 uses the
  GV100 routine (`arch/hopper/kernel_fifo_gh100.c:578-586`). Those never pass through the module's
  userspace page. (UVM's ring path: not yet checked.)
- **Hopper+ BAR1 doorbell is opt-in**: `usrmodeConstruct_IMPL` (`usermode_api.c`) maps BAR0 unless
  the client sets `bBar1Mapping`; the BAR1 view is an RM-allocated mapping, not a fixed offset. The
  same flag enables a GPU-VA ("internal MMIO") mapping of the doorbell page, i.e. GPU-originated
  doorbell rings that neither kayfabe's trap nor the module would see. ⚠ This also makes kf-trap's
  fixed `DoorbellPlacement::Bar1 { page_base: 0x9_0000 }` suspect — **to verify on a Hopper host**
  (whether libcuda sets `bBar1Mapping`). ★ Owner (2026-09-26): **the BAR1 doorbell mapping must be
  handled properly** — kayfabe must follow where RM actually places the BAR1 usermode view (and the
  GPU-VA "internal MMIO" mapping), for the trapped path and for the module alike. Tracked as its own
  work item, independent of the module.
- kayfabe already maps the host usermode page read-only into the guest (`HostPassthrough`,
  `crates/kf-trap/src/memmap.rs`), so reads are already direct; only writes exit.

## 4. kayfabe-side changes (design)

1. A setup-time memslot exposing the host doorbell page read/write, placed at a guest-physical
   address announced to the module (e.g. a vendor-specific PCI capability or a small extra BAR);
   the existing usermode page is unchanged.
2. The doorbell table: a versioned header + one atomic u64 word per guest token
   (`host_token | route | valid`), written only by kayfabe's channel plane on birth/free with a single
   atomic store; the module reads each word with one atomic load (never a torn entry, no seqlock).
   Served to the module over the virtio command set.
3. Grading: per-token `forwarded>0` can no longer come from kayfabe's doorbell counter for
   module-routed tokens. Use host-side `GP_GET` progress on the twin (authoritative: the GPU consumed
   work) and, secondarily, module counters exported read-only.

## 5. Hook point (the fragile part)

The module must substitute its page when the stock driver maps the usermode object into a process.
Candidates, most robust first: (a) a VMA post-pass — after the driver's `mmap` of the usermode
object returns, the module replaces the PTE(s) of that VMA with its own page and fault handler;
(b) ftrace/kprobe on `nvidia_mmap_helper` for the usermode offset. Both depend on the driver's
internals and need per-release maintenance; the design keeps the module optional precisely so a
driver update that breaks the hook degrades to today's trapped path, never to failure.

## 6. Safety and tests

- Bare metal (raw client): ring random, stale and foreign host tokens (another process's channel,
  one of our Translated rings) and show nothing breaks — the GPU only re-reads the target's own
  `GP_PUT`. This validates the accepted-risk assumption on each family we support.
- Guest: the 30-arm suite and the CUDA ladder with and without the module (identical verdicts), a
  token-table churn test (channels born/freed while ringing), and module-unload mid-run.

## 7. Expected gain (estimates, not measured)

Per passthrough doorbell: ~50 µs (nested exit) → ~1–3 µs (guest `#PF` + table lookup + host store).
LLM decode at ~1,008 doorbells/token: ~50–70 ms/token saved on the nested boxes (≈ the measured
gap), projecting roughly 0.7–0.9× bare metal there. On a non-nested host the exit is already
~2–6 µs, so the gain is smaller (~2–5 ms/token).

## 8. Open

UVM's doorbell path; Hopper `bBar1Mapping` and GPU-originated rings; hook maintenance per driver
release; a non-nested baseline to decide priority.

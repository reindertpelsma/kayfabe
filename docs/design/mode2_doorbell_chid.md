# Mode-2: doorbell / chid / work-submit — and the legacy-vGPU resolution

Status: design analysis, source-grounded against the 580.159.04 open driver
(research_clones/ogkm) + the GA106 (clc361/clc561) class headers. 2026-06-05.

This settles how userspace submits work to the GPU, where the doorbell lives, why
the kernel/userspace doorbell can't be trapped selectively, the chid-collision
problem for direct passthrough, and the **legacy-vGPU mode that dissolves it**.

## 1. The userspace submit path: what's memory vs what's a register

Volta+ (Ampere GA106) kernel-bypass submission:

- **USERD** (holds GP_PUT/GP_GET), **GPFIFO**, **pushbuffers**, **semaphores/notifiers**
  are ordinary **memory** objects — FB or sysmem (`kernel_channel.c:2495-2500`,
  `ADDR_FBMEM`/`ADDR_SYSMEM`). Userspace reaches them via a BAR1-mapped GPU-VA (FB)
  or a direct map (sysmem). The GPU reads/writes them through its own MMU.
- **The doorbell** is the only register piece: `NVC361_NOTIFY_CHANNEL_PENDING`
  at **+0x90** inside the **64 KiB USERMODE register block** (`clc361.h`:
  `NVC361 = 0x810000:0x81ffff`, `__SIZE = 65536`), created `ADDR_REGMEM`
  (`kernel_fifo_gv100.c:371`, base from `gpuGetRegBaseOffset(NV_REG_BASE_USERMODE)`).
  It is **not** in VRAM/FB physical — writing any FB address never rings it.
- Per submit: write GP_PUT into USERD (memory), then write the **work-submit token**
  to `usermode_base + 0x90` (register). No syscall — kernel-bypass.

The token is **structural, not cryptographic**: `{runlistId, chId}`
(`kernel_fifo_ga100.c:226-227`: `FLD_SET_DRF_NUM(_CTRL,_VF_DOORBELL,_RUNLIST_ID,..)`
`+ _VECTOR, chId`). It's a "go look at channel (runlist, chid)" hint.

## 2. The table behind the token: CHRAM + chidMgr

- **CHRAM** (Channel RAM): a HW register-indexed table per runlist
  (`chramPriBase + NV_CHRAM_CHANNEL(chId)`, `kernel_fifo_ga100.c:907`), each entry =
  enable bit + instance-block pointer. doorbell `{runlist,chid}` -> `CHRAM[chid]` ->
  instance block -> run. One physical CHRAM per runlist on the GPU, **shared by all
  clients** (host processes + all forwarded guests).
- **chidMgr** (`CHID_MGR`): the SW allocator, a per-runlist heap/bitmap
  (`_kfifoChidMgrAllocChidHeaps`), hands out **dense low integers**, bottom-up from
  `rangeLo` (`KFIFO_NUM_GSP_RESERVED_CHANNELS` reserved at the very bottom for GSP).
- `USERD_INDEX_FIXED` pins the chid: the alloc encodes `ChID % numChannelsPerUserd`
  / `ChID / numChannelsPerUserd` into USERD_INDEX_VALUE/PAGE (`kernel_channel.c:2698-2700`).
- A successful channel alloc **self-reserves** its chid (marked IN_USE until freed);
  RM won't hand it to another client meanwhile. There is **no** client-facing
  "reserve range [a,b)" API.

## 3. Doorbell page is separate from the control plane (good)

Two different BAR0 register blocks, different 4 KiB pages:

- **GSP-RPC control plane** (allocs / controls / FREE — everything `shadow_fwd`
  forwards): notified via the **NV_PGSP queue-head doorbell at 0x110c00** (GSP falcon
  block, base 0x110000). This is where the stuff we *emulate* is signalled.
- **PDB / page tables**: plain CPU MMIO writes to FB via PRAMIN/BAR — memory plane,
  not a doorbell at all.
- **Channel work submit**: the usermode VF doorbell at **0x810090**.

So you can trap {0x110000 GSP-RPC page + PRAMIN/FB} and passthrough {0x810000 channel
doorbell} — they never overlap. The only thing on the channel-doorbell page is channel
work (kernel scrubber/ctx-init AND userspace compute), not control RPCs.

## 4. Why you can't "trap only kernel rings"

- One doorbell **register**, on one **page**. EPT/MMIO trapping is page-granular and
  value-agnostic — you can't "trap only writes whose token is in the kernel set."
  It's strictly all-or-nothing: trap every ring or none.
- Kernel (CE scrubber/ctx-init) and userspace both ring the **same** register
  (`kfifoUpdateUsermodeDoorbell_GA100`: `GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_DOORBELL,
  token)`). On Ampere there is no separate privileged doorbell page (Hopper+ split it
  into `pBar1VF`/`pBar1PrivVF`; not GA106).
- Cost of trapping: a doorbell write is ~10-30 ns native vs ~1-3 us as an MMIO
  VM-exit (~30-100x). Matters for launch-heavy workloads (LLM decode rings often).

## 5. The chid-collision problem for direct passthrough

To passthrough the doorbell (zero trap), the guest's `{runlist, chid}` must hit the
*correct* host channel -> need `guest_chid == host_chid`. But guest and host run
independent chidMgrs over **dense low integers in one shared CHRAM**, so they collide
by default. And you can't pre-reserve a slice unprivileged (no reserve API;
placeholder channels occupy the chids you'd want to allocate into; free-then-realloc
races). A hard reservation needs privilege (SR-IOV/vGPU `chidOffset`, set up once).

## 6. THE RESOLUTION: legacy-vGPU mode (host allocates the chid)

`kernel_channel.c` gates chid allocation on **`IS_VIRTUAL_WITHOUT_SRIOV`** (legacy vGPU):

- **bare-metal / GSP-client / full-SRIOV** (`!IS_VIRTUAL_WITHOUT_SRIOV`, what Mode-2
  emulates today): guest calls `kchannelAllocHwID_HAL` **locally, before the RPC**
  (line 813) -> **guest picks chid** -> collision-prone (this is the
  `"Error in Allocating channel id"` path we hit with leaked 0xdead0007 channels).
- **legacy vGPU** (`IS_VIRTUAL_WITHOUT_SRIOV`): guest **skips** local chid alloc,
  **RPCs the channel alloc to the host first** ("so that instmem details can be gotten
  from it", line 833), then `// Legacy VGPU: allocate chid that the host provided`
  (line 844) adopts `pKernelChannel->ChID` **filled in by the host via the RPC**.
- line 2603 confirms the broader split: *"GSP client or full SRIOV: the guest
  independently allocs ChID and instmem"* -> i.e. legacy vGPU = **host allocates/
  describes instmem too**, not just chid.

So in legacy-vGPU posture:
1. Present the emulated GPU as legacy vGPU (`IS_VIRTUAL_WITHOUT_SRIOV`).
2. Guest channel alloc -> channel-alloc RPC -> we forward to the **real host RM**,
   which picks a genuinely **free** host chid `Z` (its own pool; never picks in-use ->
   **collision impossible by construction**, no FIXED, no reservation).
3. Return `Z` to the guest in the RPC response; guest adopts `Z`.
4. Guest writes `Z` to the doorbell -> passthrough -> host `CHRAM[Z]` = our channel.

`guest_chid == host_chid == Z` always. **Zero trap, zero collision, unprivileged,
no reservation.** And it's the Mode-2 thesis natively: host does control-plane
allocation (chid, instmem, channel construct), guest does data-plane submission direct.

## 7. Tiered model (collapses if legacy-vGPU works)

- **Tier 0 — legacy vGPU (target):** host-authoritative chid/instmem -> passthrough
  doorbell, zero trap, zero collision, unprivileged. The win.
- **Tier 1 — trap-all (fallback if not in vGPU posture):** fully unprivileged; trap
  every ring, translate guest_chid->host_chid. ~1-3 us/ring; fine for GPU-bound
  compute, stings tiny launches.
- **Tier 2 — unprivileged best-effort:** bare-metal posture + steer guest to a high
  chid range + self-reserving FIXED allocs + dynamic flip to trap-all-with-translation
  on first collision (per-channel trap is impossible — flip is global). Soft guarantee.
- **Tier 3 — privileged hard reserve:** SR-IOV/vGPU `chidOffset` partition set up once
  by a privileged bootstrap (then drop privs). Hard guarantee. Largely obviated by Tier 0.

## 8. To verify before committing to legacy-vGPU

1. How to drive the stock 580 open driver into `IS_VIRTUAL_WITHOUT_SRIOV` (advertise
   vGPU-without-SRIOV: the virtualization register — emulator currently sets
   `NV_PMC_BOOT_1` to VGPU=REAL/no-virt; flip it — plus the legacy-vGPU GSP/RPC
   handshake). Confirm it lands on *without-SRIOV*, not full-SRIOV (which returns chid
   to the guest).
2. Confirm 580 open driver doesn't gate legacy-vGPU guest support to datacenter SKUs.
3. Scope: legacy vGPU re-bases the emulation posture (more host-authoritative setup
   round-trips). Aligned with forwarding (control plane), but a real new bring-up.

Source pointers: kernel_channel.c (812/833/844/2603/2698), kernel_fifo_ga100.c
(226/884/907), kernel_fifo_gv100.c (155/371), usermode_api.c, clc361.h, clc561.h
(AMPERE_USERMODE_A=0xc561), kernel_fifo.c (805/1161/3225 chidOffset).

## 9. Full USERMODE / VF register-block layout (2026-06-05)

The public SDK header (clc361.h) exposes only TIME_0/1 + NOTIFY_CHANNEL_PENDING.
The real HW block NV_VIRTUAL_FUNCTION splits into:
- **PRIV region (VF off 0x00000-0x2FFFF, ~192KiB)** — kernel-only, PLM-gated, NOT on the
  userspace page: PRIV_ACCESS_COUNTER0_CONFIG (0x3100) + NOTIFY_BUFFER_LO/HI/GET/INFO
  (0x3108+) = UVM access counters (migration hints); FULL_PHYS_OFFSET / PHYS_OFFSET_REGION0
  (0xBBFFFF) / REGION1 (0xDBFFFF) = VF phys-offset windows.
- **USERMODE window (VF off 0x30000+, the 64KiB clc361 maps to BAR0 0x810000)** — the page
  every userspace process maps (and the kernel uses). clc361 off 0xN == VF off 0x30000+0xN:
    CFG0     @0x30000 (win 0x00) — config; USERMODE_CLASS_ID (class identity)
    TIME_0   @0x30080 (win 0x80) — PTIMER GPU nanosec clock lo
    TIME_1   @0x30084 (win 0x84) — PTIMER nanosec hi
    DOORBELL @0x30090 (win 0x90) — NOTIFY_CHANNEL_PENDING, the work-submit doorbell
    ERR_CONT @0x30094 (win 0x94) — error containment status
  Rest of the 64KiB is reserved (sparse: ~5 regs padded to a page).

DOORBELL (0x90) fields: VECTOR[11:0]=chId, RUNLIST_ID[22:16], RUNLIST_DOORBELL bit,
**GSP_DOORBELL[31:31]** (set => rings GSP, not a channel), HANDLE[31:0]=opaque token.

Passthrough implications:
- The userspace 64KiB window is clean to passthrough (CFG0 read, TIME read = real GPU
  clock, DOORBELL = the target, ERR_CONT read). Privileged access-counters are in the
  separate 0x0-0x2FFFF region, not exposed by mapping the userspace window.
- WRINKLE: GSP_DOORBELL[31] lets the SAME 0x90 register ring GSP (control-plane, wants
  TRAP) vs a channel (wants PASSTHROUGH). Per-register trapping can't split by value, so
  VERIFY the guest rings GSP via the separate GSP queue doorbell 0x110c00 (what our
  emulator observes) and only uses 0x90 for channel submit — esp. under legacy-vGPU posture.

## 10. GSP_DOORBELL wrinkle RESOLVED (2026-06-05): Blackwell-only; vGPU RPC uses a separate PRIV doorbell

Confirmed from source:
- `NV_VIRTUAL_FUNCTION_DOORBELL_GSP_DOORBELL` (bit 31) is defined ONLY in
  `blackwell/gb100/dev_vm.h` — it does NOT exist on Ampere. The GA100/GA102 doorbell
  register has no GSP bit, and `kfifoGenerateWorkSubmitToken_GA100` sets only RUNLIST_ID
  + VECTOR (bit 31 = 0). => On GA106 the userspace VF doorbell (win 0x90) is
  structurally CHANNEL-SUBMIT-ONLY; it cannot ring GSP. The §8 wrinkle does not apply.
- vGPU RPC notification uses a SEPARATE privileged doorbell: `NV_VIRTUAL_FUNCTION_PRIV_DOORBELL`
  @ 0x2200 (inside the PRIV region 0x0-0x2FFFF, kernel-mapped, NOT the userspace page).
  `vgpu/arch/ampere/rpcga102.c:49` rings it: `GPU_VREG_WR32(.., PRIV_DOORBELL, doorbellToken)`
  with NV_DOORBELL_NOTIFY_LEAF_VF_RPC_* tokens (SETUP/MESSAGE_REQUEST, nv_sriov_defines.h).
  Also used by intr_sriov_tu102.c, kernel_gsp_gh100.c, kernel_hostvgpudeviceapi.c
  (TRIGGER_PRIV_DOORBELL).

=> In legacy-vGPU posture the control/data doorbell split is HARDWARE-NATIVE, by privilege/page:
  - CONTROL (vGPU RPC -> host: chid/channel/instmem allocs): PRIV_DOORBELL 0x2200 (PRIV
    region, privileged page) -> TRAP. This is the interception point for the vGPU control
    plane (where we receive the RPCs to forward to the host RM).
  - DATA (channel work submit): userspace VF doorbell win 0x90 (token {runlist,chid},
    no GSP bit on Ampere) -> PASSTHROUGH.
  Different pages by privilege => the per-page trap/passthrough split is native, not engineered.
This RESOLVES §8 verify-item #2 and strengthens §6: legacy-vGPU gives both host-authoritative
chid/instmem AND a clean control(0x2200)/data(0x90) doorbell separation.

## 11. Blackwell forward-look (2026-06-05): doorbell split survives; real risk is legacy-vGPU support

Verified from gb100/dev_vm.h + shared vgpu rpc:
- Blackwell KEEPS the separate privileged doorbell: NV_VIRTUAL_FUNCTION_PRIV_DOORBELL @
  0x2200 (PRIV region), field renamed to CPU_NOTIFICATION[31]+VECTOR[11:0]. The vGPU RPC
  path is shared (only vgpu/arch/ampere/rpcga102.c exists; rings PRIV_DOORBELL). So a
  Blackwell vGPU guest still signals control-plane RPC via 0x2200 (separate page) — same
  as Ampere.
- The userspace doorbell (0x30090) ADDS GSP_DOORBELL[31] (vs Ampere has none). But channel
  submit (gb202 kfifo) sets RUNLIST_DOORBELL, not GSP. GSP_DOORBELL is a BARE-METAL
  GSP-client optimization (ring GSP directly from the userspace doorbell instead of the
  0x110c00 queue head), NOT the vGPU control path.
=> In vGPU posture the control/data page split SURVIVES on Blackwell: control=PRIV_DOORBELL
   0x2200 (trap), data=userspace doorbell 0x30090 (passthrough). GSP_DOORBELL[31] only bites
   if we present BARE-METAL GSP-client posture; staying in vGPU posture keeps the userspace
   doorbell channel-only. (Confirm at bring-up: no vGPU-mode path sets GSP_DOORBELL on 0x30090.)

The REAL Blackwell question is chid allocation, not the doorbell: verify whether
IS_VIRTUAL_WITHOUT_SRIOV (legacy vGPU) is still supported on Blackwell, or whether it forces
full-SRIOV vGPU (where the guest allocs chid itself, line 2603).
- legacy-vGPU survives -> identical to Ampere (host-allocates chid).
- full-SRIOV mandated -> chid guest-side but in a HW-partitioned VF chram slice (chidOffset,
  vChid->sChid). Passthrough then needs either real host SR-IOV (HW does the offset in the
  VF doorbell) or emulated VF-partition + trap+translate.
Universal backstop: trap-all + guest->host chid translation works on ANY arch/posture (slower
tier), so Blackwell is never blocked — worst case runs trap-all until the posture is sorted.

## 12. CORRECTION (2026-06-05): legacy-vGPU is NOT reachable with the stock OPEN driver

§6 over-claimed. Verified the open driver's virtual-detection:
- `gpumgrGetGpuHalFactorOfVirtual` (gpu_mgr.c:1003): sets isVirtual=TRUE **only** if
  NV_PMC_BOOT_1.VGPU == _VF. The _PV value (0x1, para-virtual = legacy) is NOT recognized
  -> falls through to isVirtual=FALSE (treated as BARE METAL).
- NV_PMC_BOOT_1_VGPU (nv_ref.h:146) is a 2-bit field: _REAL=0, _PV=1, _VF=2.
- gpu.c:388 sets bIsVirtualWithSriov = (VGPU==_VF) within the IS_VIRTUAL case; the else
  (legacy) branch is unreachable because IS_VIRTUAL already requires _VF.
- gpuDetermineVirtualMode (gpu.c:4552): NV_ASSERT_OR_RETURN(isVirtual == (VGPU==VF),
  NV_ERR_INVALID_STATE) — release-effective; enforces the coupling.
=> isVirtual <=> VGPU==VF <=> bIsVirtualWithSriov. There is NO emulated-register state that
   yields "virtual without SRIOV". The registry key NV_REG_STR_RM_SET_SRIOV_MODE
   (gpu_registry.c:105) sets the HOST/PF-side bSriovEnabled, NOT the guest virtual-mode.
   The IS_VIRTUAL_WITHOUT_SRIOV code paths exist (shared w/ the proprietary driver, which
   supported pre-Ampere para-virtual vGPU) but are DEAD in the open build.

CONSEQUENCE: §6's host-allocates-chid resolution requires a 1-line GUEST-DRIVER PATCH
(accept _PV in gpumgrGetGpuHalFactorOfVirtual, or force bIsVirtualWithSriov=FALSE at
gpu.c:388) -> debug/Linux-demo only; breaks the stock-driver (Windows/closed-KMD) thesis.

REVISED production options (Tier model §7 stands; Tier 0 needs a guest patch):
- Full-SRIOV-VF emulation (present VGPU=_VF): guest allocs chid in its VF partition (line
  2603); translate vChid->sChid (trap) OR use REAL host SR-IOV so the per-VF doorbell does
  the offset in HW (zero-trap, but needs SR-IOV unlocked on the host GA106).
- Bare-metal (current Mode-2): high-range chid steering + per-collision flip to trap-all.
- Trap-all: universal floor.
The dig confirmed full-SRIOV-VF is the only stock-driver "vGPU" posture -> real host SR-IOV
(if unlockable on consumer GA106) is the path that yields HW chid-partition + HW doorbell
translation for free; otherwise chid translation is a bounded per-channel cost, not the
original collision nightmare.

## 13. Doorbell transport DECISION (2026-06-05): read-only USERMODE page, trap-on-write

The USERMODE 64KiB page is mostly reads (PTIMER TIME_0/1, CFG0, ERR_CONT) + one
write target (DOORBELL @0x90). So:

- Map the host's USERMODE register page into the guest as a **KVM_MEM_READONLY memslot**
  (backed by QEMU's mmap of the host AMPERE_USERMODE_A object — same mmap-into-guest
  machinery as Mode-1's GPA window, +the RO flag).
  - **Reads → native** (no VM-exit): guest -> EPT -> host USERMODE registers. The GPU
    nanosecond clock (PTIMER) and CFG0/ERR_CONT are read at hardware speed. Real win:
    timestamp reads (CUDA events/profiling) cost nothing.
  - **Writes → KVM_EXIT_MMIO -> QEMU** (the only writes are doorbell rings).
- **Fast write handler** (the slightly-hot path): extract {runlistId, vChid} from the
  token, O(1) array lookup vChid->host sChid, store {runlistId, sChid} to QEMU's RW
  mapping of the host doorbell. ~sub-us CPU; VM-exit (~1-3us) dominates. Keep it
  lockless/per-vCPU; no allocation; a flat per-runlist host_chid[] table.
- **Adaptive**: if chid-identity holds (single-tenant, no host-channel collision, or real
  SR-IOV) map the page **RW -> zero-trap doorbell**; else **RO -> trap-writes-only**.
  Reads stay native in both modes. This is the perf knob: zero-trap best case, cheap
  trap-per-submission-batch fallback (amortized for GPU-bound work, see §perf below).

Perf framing: a doorbell ring submits a BATCH (CUDA-graph / coalesced GPFIFO), not one
per kernel; ~1-3us/trap is hidden behind the GPU executing the batch. Mode-1 shipped a
+45us/launch tax (15-45x larger) at throughput parity on matmul+LLM decode because
GPU-bound work hides submit overhead. So trap-write is a knob, not a showstopper.

This makes the doorbell a clean instance of the data-plane object model (next doc): a
"special" register-page object, RO-mapped, with a write-fault handler = chid-translate +
forward. It is NOT GPU-physical-backed (corrects the older brainstorm).

## §14 Doorbell trap as channel demux: fake kernel channels, forward userspace (user, 2026-06-05)

Because chids need translation (guest vChid namespace != host sChid), the doorbell write MUST be
trapped (no zero-trap/identity shortcut in the general case). That mandatory trap is also a free
per-kick DEMUX point — at each doorbell we know which channel is rung (token->chid). Use it to
split work:

- **Kernel-internal channels** (the RM scrubber channel, other RM bookkeeping): SIMULATE
  completion at the trap — write the channel's completion semaphore + raise the expected
  NV906F_NON_STALL_INTERRUPT — WITHOUT forwarding to the host. Justified ONLY when the channel's
  effect is REDUNDANT in our model: e.g. the FB scrub-to-zero is already done by our host
  RM_ALLOC (RM scrubs every vidmem alloc for security), so re-running it is a no-op; we just owe
  the guest init state machine the completion signal. This resolves the "scrub optional but the
  scrubber-channel COMPLETION must still fire" caveat: fake the completion here.
- **Userspace channels** (libcuda compute): FORWARD the kick (translate chid, write host
  doorbell). The work runs on the real GPU at full speed, untouched — the trap is a lightweight
  forward, not interpretation.

THE LINE (keeps [[mode2-real-forward-not-fake]] intact): fake ONLY channels whose work is
genuinely redundant/unneeded; NEVER fake a channel that produces GPU state the guest later
CONSUMES. Sharp example: the GR golden-context load (FECS) IS the content libcuda reads -> must be
REAL. It falls out correctly for free: FECS golden-load is triggered by the USERSPACE GR channel's
first run (context switch), which we forward -> the host GPU loads the golden ctx for real into the
double-mmapped ctx buffer. So: scrubber=fake; GR/compute=forward; FECS-load=real (via forwarding
the userspace chan). Classification: tag each channel kernel|user at alloc by its owning RM client
(RM-internal vs libcuda's client) — already tracked in the channel table; the chid table carries
the bit. Doorbell handler (plan item 5) = translate chid -> if kernel-internal: fake-complete;
else: write host doorbell.

## §15 BAR0 register plane: host-map the USERMODE/PTIMER window RO; keep boot/GSP emulated (user 2026-06-05)

Pivot the USERMODE doorbell + PTIMER window of BAR0 from full software emulation to a host-backed
RO memslot (part of the refactor; the register-plane special gpu_memory_object):
- Back it with the REAL host USERMODE mapping: host AMPERE_USERMODE_A alloc -> RM_MAP_MEMORY ->
  host VA -> install as a KVM_MEM_READONLY memslot at the guest BAR0 sub-region GPA.
- REMOVE THE WRITE BIT: reads native (no exit) -> the high-precision PTIMER nanosecond clock is
  read at full speed directly from the real host GPU registers; doorbell WRITES fault
  (KVM_EXIT_MMIO) -> handler -> chid translate (vChid->sChid) -> write the host doorbell. A single
  KVM_MEM_READONLY memslot gives both (reads from backing, writes exit).

SCOPE (sharp edge): host-map-RO ONLY the USERMODE+PTIMER window. The rest of BAR0 — boot/GSP/PMC/
WPR2/GFW_BOOT/control regs + the GSP-RPC doorbell 0x110c00 — MUST stay EMULATED, because those
carry our fake-the-boot identity/state; the guest must read OUR emulated values, not the host
GPU's live registers, or fake-the-boot breaks. So BAR0 is a MIX:
  - boot/GSP/PMC/control       -> emulated (MMIO-trapped, our fake-boot state)
  - USERMODE doorbell + PTIMER -> host-mapped RO special object (native reads, trapped doorbell)
This is the register-plane analog of the memory-plane double-mmap (same host-ioctl-backed model,
mode=special with a write fault handler instead of mode=physical).

---

## §16 — Doorbell/chid demux: measured findings + build plan (2026-06-05)

### Measured doorbell token format (GA106, cup2 m2exec run)
The guest writes its work-submit token to the VF USERMODE doorbell (`NVKVM_VF_DOORBELL`,
`NV_USERMODE_NOTIFY_CHANNEL_PENDING` @0x90). Captured distinct tokens during cuCtxCreate:
```
token=0x00000004   -> runlist 0, chid 4
token=0x00010008   -> runlist 1, chid 8
token=0x00010001   -> runlist 1, chid 1
```
=> **token = (runlist << 16) | chid**. Three channels submit work + are polled for completion
during cuCtxCreate (confirms the blocker is multi-channel, not one). The guest assigns these
vChids in ITS channel space; the host RM assigns DIFFERENT sChids to the forwarded channels →
the token must be TRANSLATED (vrunlist:vChid -> srunlist:sChid) before writing the host doorbell.

### Ownership model (user direction, 2026-06-05) — what we DO vs DON'T do
- **GR context switching = the HOST kernel/GPU's job.** We do NOT manage ctxsw, golden context,
  or ctx-buffer content. The host RM/GSP already builds a valid GR context when we forward the
  0xc7c0 alloc (st=0x51 self-mapped ctx buffers). The guest's GR ctx buffers are
  MEMDESC_FLAGS_GPU_PRIVILEGED (kernel-only) → guest USERSPACE never observes them, so we only
  owe the guest KERNEL a non-faulting backed dummy page at the (deterministic) GR VA to satisfy
  its open-source checks. We TRUST the host on channel execution.
- **Pushbuffer + GPFIFO live in guest RAM (sysmem).** To let the host GPU run them, pin the
  guest-RAM pages via OS_DESCRIPTOR (host nvidia ioctl, proven in item-4) and map_dma them FIXED
  into the host channel's VAS at the guest VAs. These are chid-INDEPENDENT in content → shareable
  to the host directly. USERD (GP_PUT) + the doorbell token are chid-DEPENDENT → translate.

### Build plan (item 5 — the cuCtxCreate keystone), in order
1. **chid/token table.** At each forwarded GPFIFO channel alloc (class ..6F), record
   {guest_chan_hObj, gpfifo_va, vChid/vrunlist, host_chan_hObj, host_token (via 0xc36f0108 on the
   host channel)}. Decode vChid/vrunlist from the channel's runlist slot or the token the guest
   later writes; sChid/sToken from the host.
2. **Working-set forward (per channel with new work).** OS_DESCRIPTOR-pin + map_dma FIXED the
   guest-RAM pushbuffers + GPFIFO + referenced data + completion-semaphore page into the host
   channel VAS at the guest VAs (M5.10 re-sweep handles vidmem leaves; guest-RAM via item-4).
   Completion semaphore 0x2efbaf000 is reached via the BAR2 root 0x2f3392000 (NOT in chan_vas[])
   — add that root to the enum so its leaf is backed.
3. **Doorbell demux.** Trap the guest USERMODE write (already trapped), decode token ->
   (vrunlist,vChid) -> table -> host channel -> write the host's token to the host USERMODE.
   (Optimization later: make USERMODE a KVM_MEM_READONLY memslot so reads/PTIMER are native and
   only writes fault.)
4. **Gate the ring** on "this channel's working set fully mapped" (naive ring today -> cuInit=999
   because the host faults on unmapped referenced VAs).
5. Host runs the channel -> writes GP_GET + the completion semaphore -> guest poll satisfied ->
   cuCtxCreate proceeds. Verify via host nvidia-smi util (real work), CRASHWIN 0x2efbaf000 0->nonzero.

Quarantine the QEMU-side `nvkvm_chan_execute()` pushbuffer-parse/sema-fake path during the real
build (it masks whether the host actually ran the work).

### §16.1 — MEASURED: chid table + the demux decision (2026-06-05, GA106 cup2 m2exec)
Built the per-channel host-token table (M5.12): `shadow_fwd` creates the host channel with the SAME
hObject, so `0xc36f0108` (GET_WORK_SUBMIT_TOKEN) on the guest channel handle returns the HOST token.
Measured host tokens (sequential per runlist):
```
gpfifo 0x121010000 -> 0x00000004 (rl0 chid4)   0x121040000 -> 0x05   0x121070000 -> 0x06
0x1210a0000 -> 0x07   0x1210d0000 -> 0x00010008 (rl1 chid8)   0x121100000 -> 0x10009
0x121130000 -> 0x0002000a (rl2 chid10)   0x121160000 -> 0x2000b   GR 0x200200000 -> 0x0c
```
Guest doorbell tokens written during cuCtxCreate: `0x4`, `0x10008`, `0x10001`.
- `0x4`  == host chan token (gpfifo 0x121010000) ✅
- `0x10008` == host chan token (gpfifo 0x1210d0000) ✅
- `0x10001` — **NO host token equals it** ❌

**DECISION: doorbell pass-through is INCORRECT.** Guest vChid and host sChid usually coincide
(both RMs allocate chids sequentially in the same order) but NOT always (`0x10001` diverges). So we
must NOT forward the guest's token verbatim. Use **GP_PUT-driven demux**: at the trapped doorbell,
scan all forwarded channels' USERD GP_PUT, and for each that advanced since last ring, ring ITS
`host_token` (from the M5.12 table). This needs no vChid→sChid decode and is robust to the divergence.

Known gap: chan[0,1,2] (the 4096/32-ent early channels) didn't get tokens fetched (fetch err /
gpfifo_va=0) — revisit when wiring the ring.

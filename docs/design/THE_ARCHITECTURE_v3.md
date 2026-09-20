# kayfabe v3 — the architecture after the owner's review

**STATUS: LIVE, 2026-09-20 (w820). Supersedes `THE_ARCHITECTURE_v2.md`**, which this document
keeps only as the record of what was proposed before the review corrected it.

⊘ **w820 rewrote §2 entirely** around a second privilege boundary the owner named: the one
**inside the guest**, between unprivileged guest userspace and guest root. The doorbell hint
queue and its "inspect all doorbells" fallback are **deleted**, not fixed; per-vCPU register
rings are **refuted**; the worker wakeup protocol is specified with its orderings. §§1, 8 and 10
were updated to match. Everything else is unchanged from w819.

⊘ Written from the owner's point-by-point review. Where the review corrected me, the
correction is marked **[CORRECTED]** with what I had wrong — those are the load-bearing edits,
not footnotes.

---

## 0. ★★★★★ THE SEVEN AXES — every constant in this design varies along at least one

**[NEW, w821]** `[owner, 2026-09-20]` listed the axes this product must span. They are the
reason almost nothing in Part 2 may be a literal in our source, and they are **prior to** every
other section: a design that is correct on one point of this lattice and cannot be extended to
the rest is a retrofit waiting to happen.

| tag | axis | posture | how variation is absorbed |
|---|---|---|---|
| **K** | **guest kernel** version | ⊘ all major versions | ⊘ **[CORRECTED w821]** *not* via struct layouts — RM and UVM structs are NVIDIA-defined and kernel-independent. What K actually moves that we see is **DMA addressing**: a guest booted with a **vIOMMU** hands us **IOVAs, not GPAs**, in every sysmem address we are given. ⇒ K × V, and unhandled |
| **Dg** | **guest driver** version | ⊘ all major versions | **generated** from that version's headers; a per-version **profile entry**, never `if version ==` |
| **Dh** | **host driver** version | ⊘ all major versions, **decoupled from `Dg`** | we **author** every host call; our host-verb signatures do not accept a guest flag word |
| **A** | **GPU architecture** | ★ Turing and newer — ⊘ **FORCED, not chosen** (below) | a **format family** descriptor (four families span Turing→Blackwell) |
| **die** | **GPU die** within an arch | ⊘ any die | derived per die, **maintained per family**; a new die is a descriptor, not a code path |
| **V** | **VMM** | ◐ **the one axis where a version floor is legitimate** — QEMU, Cloud Hypervisor | the core is VMM-agnostic; the VMM shim is the only place that knows |
| **OS** | **guest OS** | Linux **and** Windows | ◐ **[SURVEYED w821 — see Part 3.]** No longer zero-coverage. ⊘ GSP is **default-on for Turing+ on every OS** (the shared core has no OS branch) — an early claim that Windows defaults it off was **retracted**. What does bite: under WDDM the **OS owns the page tables** and the PDE-update path is never RPC'd to us; TDR is a hard **2 s** vs Linux's 4/30 s; **UVM does not exist**. ★ TCC collapses most of it, and `bGspNocatEnabled` is a one-comparison Windows detector |

⊘ **`Dg` and `Dh` are TWO axes, not one.** The operator chooses the guest driver; we do not.
A design that assumes they match is a defect. ★ This is the structural reason for
`author_host_flags_never_forward_them`: forwarding a guest-supplied flag word into a host RM
call is not merely risky, it is **an implicit assertion that the two versions agree**.

⊘ **Host OS is Linux only** — the one scope relaxation, taken deliberately.

### 0.0.1 ⊘⊘⊘ The Turing floor is an ARCHITECTURAL BOUNDARY, not a scoping decision

**[w821]** `[owner]` *"pre-Turing it can't be used."* ★ Correct, and it reclassifies the `A`-axis
floor. `_gpumgrIsRmFirmwareCapableChip` is exactly:

```c
return (decodePmcBoot42Architecture(pmcBoot42) >= NV_PMC_BOOT_42_ARCHITECTURE_TU100);
```

⇒ **Below Turing there is no GSP at all.** A pre-Turing guest driver does not speak this protocol
*at any level*: it runs **monolithic RM** and drives the hardware directly, with no command queue,
no RPC envelope, and no firmware to impersonate. ★★★ So *"we are the GSP"* is not merely harder
there — **it is meaningless**, and no amount of engineering moves the floor.

⇒ The floor should be read as *"**this** architecture does not exist below Turing"*, not as
*"we chose to start at Turing"*.

⊘⊘ **[AMENDED w821, the same day — the floor is per-PLANE, and there is a second plane.]** I wrote
that this posture *"cannot be revisited under a product argument"*. ★ **It can — by adding a plane
rather than by moving the floor.** `[owner]` has ruled a **GSP-disabled mode** open
(`THE_WINDOWS_AXIS.md` §10), driving the hardware through registers as a monolithic driver does.
That plane has **no GSP floor at all**, because it never impersonates firmware.

| plane | floor | why | oracle |
|---|---|---|---|
| **GSP** (★ **priority**) | **Turing+**, architectural | no GSP exists below it | **ogkm — NVIDIA source** |
| **no-GSP** | ⊘ **none** | drives registers directly | nouveau + `mmiotrace` |

★★★ `[owner]` **GSP remains the priority target** — *"it will be our more stable version as we
have more source available."* ⇒ The no-GSP plane is **additive reach, never a replacement**, and
where the two share machinery — the whole of §2 — **the GSP plane sets the shape.**

⚠ So the correction to my correction: *"Turing+ is architectural"* is true **of the GSP plane and
only of it**. Stating it as a property of the product was the error — the same shape as every
other *"true of the wrong population"* finding in this document.

★ **And it separates two gates this document had been blurring:**

| gate | what it tests | where |
|---|---|---|
| **capability** | ⊘ **hardware** — is there a GSP on this die at all? `arch >= TU100` | `_gpumgrIsRmFirmwareCapableChip` |
| **default-on policy** | ◐ *within* capable hardware, should firmware be used by default? On Windows this is **per-SKU** (`devId`, `ssId`, WS/server, TCC/MCDM) | `gpumgrIsDeviceRmFirmwareCapable` + `WindowsFirmwarePolicyArg` |

⇒ The Windows uncertainty in Part 3 §1 lives **entirely inside the second gate**. It has no bearing
on the first, and therefore none on our floor.

### 0.1 The axis band for (Dg, Dh) is a DIAGONAL, not a product

`[owner ruling, 2026-08-09]` We owe NVIDIA vGPU's own interoperability policy and no more:
exact match; same major branch different minor; and **n−1** (a newer host supports the previous
major or LTS branch). ⇒ A bounded window, not every pair.

★ What that **obliges us to build**: we must know *both* versions and be able to say
*"this pair is outside the band"* — **refused by name**. ⊘ A compatibility policy that cannot
state an out-of-band pair is not a policy, it is a hope, and it decays into the
`0x56 is the forgiven status` trap where a wrong configuration simply runs on.

⊘ **[CORRECTED w821] The band is not 2-D.** An n−1 guest driver must also **support the die**, or
it never binds at all — so the refusal key is **`(Dg, Dh, die)`**, not `(Dg, Dh)`.
⚠ And a third interaction the section missed: **`Dg × A`** — the guest supplies its *own* firmware
image and protected-region metadata, whose layout our fake GSP must accept. That is a guest-driver
artifact whose shape is architecture-dependent, and it reaches us as data we must parse.

### 0.2 ★★★ The axes are not evenly distributed across the four planes

This is the useful part, and the surveys in Part 2 measured it rather than assuming it:

| plane | dominant axis | evidence |
|---|---|---|
| **GSP RPC function *numbers*** | ★ **stable** across `Dg` in the measured window | the `NV_VGPU_MSG_FUNCTION_*` enum is **byte-identical between 580.159.04 and 610.43.02** for every id we use; 610 only *appends* (`rpc_global_enums.h`) |
| ⊘ **GSP RPC *payload structs*** | ⊘ **`Dg`, and they break** | **[CORRECTED w821 — the row above was true of the wrong population, which is this document's own named failure mode.]** The *id* is stable; the *body behind it* is not. `rpc_alloc_object_v2B_04 → v2C_01`, `rpc_map_memory_dma_v03_00 → v2C_05`, **1 895 changed lines** in `generated/g_rpc-structures.h`. ⇒ Same posture as RM controls, not a stable plane |
| **GSP RPC message framing** | ⊘ **`Dg`, and it BREAKS** | the per-element header is **48 bytes** at 580 (`authTag`/`aad`/`checkSum@32`/`seqNum@36`/`elemCount@40`) and **16 bytes** at 610 (`mctpHeader`/`nvdmHeader`/`checkSum@8`/`seqNum@12`) — `message_queue_priv.h:43-51` vs `:52-67` |
| **RM control command numbers** | `Dg` (additive), **struct layouts break** | FINN-generated; the *number* is stable, the *parameter struct* is versioned |
| **BAR0 register offsets** | **A** and **die** | `NV_PGSP_QUEUE_HEAD = 0x110c00` from `ampere/ga102/dev_gsp.h`; the doorbell is `0x30090` on Turing, Ampere **and** Blackwell |
| ⊘⊘⊘ **Which BAR carries the doorbell** | **A** — *and it changes which trap plane exists* | **[MISSING until w821, and it is the worst kind of omission: an axis that moves the mechanism, not a constant.]** Ampere/Ada: BAR0. Hopper+: unprivileged clients map it over **BAR1** (`bBar1Mapping`, not privilege-gated), which v3 §2 does not trap. See §2.1 |
| **Doorbell token encoding** | ⊘ **die**, within one arch | `NV_CTRL_VF_DOORBELL` field widths are per-arch swref, and GB202 sets bit 30 where Ampere does not |
| **Channel / engine class ids** | **A**, ⊘ **and die** | `TURING_CHANNEL_GPFIFO_A 0xC46F` (the floor arch) → `AMPERE_ 0xC56F` → `HOPPER_ 0xC86F` → `BLACKWELL_ 0xC96F`. ⊘ **[CORRECTED w821]** not purely per-arch: `BLACKWELL_CHANNEL_GPFIFO_B 0xCA6F` exists on GB202/203/205/206 while GB100 has only `_A` — **die-level inside one architecture** |
| **Pushbuffer method encoding** | ★ **remarkably stable** | `NVC56F_DMA_SEC_OP`/`METHOD_ADDRESS`/`_SUBCHANNEL`/`_COUNT` is bit-identical to `NVC36F_*`; the GPFIFO entry differs only in `PRIV` (dropped at C56F) and `INVAL_SCOPE` (added) |
| **Page-table format** | ⊘ **A**, four families | Turing binds `kgmmuFmtInitLevels_GP10X`; our one built format is `_GA10X` |

⇒ ★★★ **The two planes that break hardest are the two the guest drives most**: RPC framing
(`Dg`) and page-table format (`A`). Both are already modelled as **descriptors** rather than
code paths — `ElementLayout { hdr_size, checksum_off, seqnum_off, elem_count_off, TransportHdr }`
and `GmmuFmt` — and that is the shape every other axis-carrying fact should take.

★ **The 580/610 element header is the worked example to argue from.** It is a structure whose
*size and every field offset* changed between two supported guest driver versions, and absorbing
it as data rather than as a branch is the right shape.

⊘⊘ **[CORRECTED w821 — but the descriptor as it stands does NOT absorb the pair it cites, so the
example proves less than I claimed.]** Three things it cannot express:
`elemCount` **no longer exists** at 610 (the count is derived from `length`), so an
`elem_count_off: Option<usize>` models its absence but not its *replacement*; `hdr_size` is
**runtime**-conditional, not per-version — Confidential Compute adds an encryption tag to every
element header; and 610 **validates** the `mctpHeader` version and `nvdmHeader` vendor on every
element it receives, which is behaviour, not layout. ⇒ The honest claim is that the descriptor
absorbs *offsets*, and that a `Dg` break can also move **derivation** and **validation** — which
a layout descriptor cannot hold at all.

★ **And a `Dg` break that is neither layout nor number:** 610 reads a GSP-RM **heartbeat** from a
mailbox register when an RPC times out, to classify the failure; 580 has no such path. ⇒ A slow
worker gets a *different and fatal* diagnosis at 610. **A version axis can add a new way to be
judged, not just a new way to be parsed.**

### 0.3 ⊘ What this forbids, concretely

1. ⊘ **No literal in a comparison.** `derive_per_die_maintain_per_family`. Nothing may test a
   raw offset, class id or command number against a hardcoded constant.
2. ⊘ **No C parsed with regex or sed.** Generate from the headers with a real parser front end;
   `[owner]` *"That will age better."*
3. ⊘ **No capture-derived table treated as truth.** A table captured from one driver on one die
   **expires as a vendor regression** — and this tree measured that 11 of 56 captured rows were
   empty and *every empty one checked against hardware was contradicted*.
4. ⊘ **No guest flag word forwarded to a host call.** It is a `Dg`↔`Dh` coupling in disguise.
5. ⊘ **No "Linux-shaped" assumption left unmarked.** Until a Windows guest runs, every
   guest-side inference in this document is **`OS`-limited**, and Part 2 marks the ones that are.

⚠ **Where this document is weakest on the axes**, stated plainly so it is not discovered later:
the constant-level tables in Part 2 were surveyed against **Ampere GA10x with a Linux guest**.
The axis column in each table says what *should* vary; it does **not** certify that each entry
has been exercised on a second point of that axis.

---

## 1. The shape

**One process.** The VMM (QEMU, or later any hypervisor) with the kayfabe core linked in.
No isolate children, no scratchpad process, no IPC, no sandbox plane of our own.

**Two kinds of thread besides the vCPUs:**

| thread | count | job |
|---|---|---|
| **worker** | **N, capped at 255** | the doorbell plane: scan the token table, claim a token, run the managed channel's work |
| **register drainer** | **1** | ★ **[NEW w821]** drains the privileged MPSC ring **in reservation order** — GSP RPC kicks, interrupt-tree writes, TLB-invalidate triggers. ⊘ It must be *one* thread: "an ordered ring drained by N workers" is not ordered |
| **VA manager** | 1 | ★ *new.* One synchronous thread doing **all** `mmap`/`munmap` — page-table diffs, BAR windows, channel mappings. It replaces the scratchpad isolate. |

⚠ **[w820]** The worker cap is **structural, asserted at startup** — not a configuration
convention. `workers_polling` is 8 bits of `worker_vcpu_poll` (§2.4) and a wrap would read
**0 while workers are polling**, which is the lost wakeup the protocol exists to prevent.

★ The VA manager exists because mapping is the one activity that must be serialized and must
never happen on a vCPU. Giving it a thread rather than a lock makes "who may map" a structural
answer instead of a rule.

⊘ **Sandboxing is not ours.** Sandbox the whole hypervisor process — a container, or the
hypervisor's own jail. `[owner, 2026-09-20]` *"most security bugs didn't even come from the
isolates… the entire isolate plane is a new plane that needs sharing fds and handles and stuff
back and forth… It by itself is a source of many bugs."* ★ This session supplied the proof:
`barmirror.rs:1988` does a **cross-process IPC round trip on a vCPU while holding a lock** —
a defect that cannot exist without the isolate plane. Add the 1.62 s spawn stall, `Wedged`
(a failure mode that exists only because there is a socket), the fd passing, cancel-by-signal
and the foreign-handle gate, and the plane's bug surface exceeds what it protects.

⊘ **`Proc` stops existing.** **[CORRECTED]** A process is not a GPU concept. v3 speaks only in
**channels**, **VA spaces** and **RM objects**. With isolates gone the whole per-proc
container, its arena, its pending-release queue and the dup-connected-component analysis go
with it.

---

## 2. The vCPU trap path

Only **BAR0**, only **writes**. BAR1/BAR2 are never trapped — ensuring that is a requirement,
not an optimisation. The entry point is one function, `nvkvm_trap_bar0_write`.

### 2.1 ★★★★★ The privilege boundary INSIDE the guest is the organising idea

**[NEW, w820]** `[owner, 2026-09-20]` *"the doorbell is adversarial also to the guest itself,
since unprivileged guest userspace writes to it (that's below guest kernel) … It's not about
the guest corrupting itself from the kernel/root in the guest, it's about a sandboxed/container
process in the guest corrupting the root in the guest."*

This is not hardening bolted onto the design. It **splits the trap path in two**, and every
property each half needs is derived from which side of the line it sits on:

| plane | who can write it | what it may therefore be |
|---|---|---|
| **doorbell** (usermode window) | **unprivileged guest userspace** | fixed-size, idempotent, non-degradable, needs no identity ⇒ **a bit table, no queue** |
| **privileged registers** (everything else in BAR0) | **guest root only** (RM) | an ordered ring that must not overflow — a guest root that overflows it harms only itself |

⇒ The test for any future trapped page is one question: **can unprivileged guest userspace
write it?** If yes it gets no queue, no degradation mode and no shared structure. If that is
not achievable for some page, we do not map it.

Guest-root-corrupts-guest-root is inside the guest's own trust model and not ours to defend.
Guest-container-corrupts-guest-root is the value proposition.

#### Why no check at the trap can fix this — three facts from ogkm

1. **The doorbell page is ONE page per GPU, aliased to every client.** `usrmodeConstruct_IMPL`
   takes `pMemDesc = pKernelFifo->pRegVF` (`usermode_api.c:47`) — a single per-GPU `ADDR_REGMEM`
   memdesc (`kernel_fifo_gv100.c:370`), 64 KiB (`NVC361_NV_USERMODE__SIZE`), doorbell at `+0x090`.
   `usrmodeCanCopy` is `NV_TRUE`, so it is **DUP'd, not copied**. There is **no per-client and no
   per-channel doorbell page on Ampere.** Hopper adds `bPriv`, gated on `RS_PRIV_LEVEL_KERNEL`,
   but that selects BAR1-vs-BAR0 *mapping*, not which channels you may ring.
   ★ RM waives its own privilege check on this window **by name**:
   `memdescSetFlag(pRegVF, MEMDESC_FLAGS_SKIP_REGMEM_PRIV_CHECK, NV_TRUE)` (`:374`). Handing a
   BAR0 register window to unprivileged userspace is deliberate, documented NVIDIA behaviour.
2. **The token carries no capability.** `kfifoGenerateWorkSubmitTokenHal_GA100`
   (`kernel_fifo_ga100.c:224`) is a plain concatenation of `runlistId` and `chId`. RM's comment
   *"Caller cannot make assumption about this handle"* is a request, not an enforcement: the
   space is small enough to enumerate by counting.
3. **The trap carries no attributable identity**, and this is the load-bearing claim, so it is
   cited rather than asserted. We get a vCPU, a GPA and 32 bits the attacker chose — nothing says
   *who* rang. ★ **The two mappings provably coincide:** the usermode window's address is a
   **BAR0-relative register offset** (`kfifoGetUsermodeMapInfo_GV100` returns
   `gpuGetRegBaseOffset_HAL(NV_REG_BASE_USERMODE)`, `kernel_fifo_gv100.c:165`), and the driver's
   own mmap gate admits it precisely *because* it lies inside the register BAR —
   `IS_REG_OFFSET` tests `offset >= nv->regs->cpu_address` and within `nv->regs->size`
   (`kernel-open/common/inc/nv.h:854`), where `nv->regs->cpu_address` is BAR0's physical base,
   which under KVM is a **guest**-physical address. The guest kernel reaches the same registers
   by `ioremap` of that same base. ⇒ Both mappings land in **one KVM memslot at one GPA**.
   ⊘⊘ **[CORRECTED w821 — my reasoning here was wrong, though the conclusion survives.]** I wrote
   that the trap *"cannot distinguish kernel from userspace. Not with more checks. Ever."*
   **False**: an MMIO exit hands us the vCPU, and its **CPL is readable** (`KVM_GET_SREGS`,
   ~1 µs). Kernel-vs-user **is** distinguishable. ⇒ The load-bearing unavailability is not
   privilege, it is **process identity**: nothing tells us *which* of the guest's unprivileged
   processes rang, and that is what an authorization decision would need. The conclusion — the
   ring cannot be authorized at the trap — holds, for that reason and not for the one I gave.
   ⚠ And CPL is not free to consult on a hot path; it is an argument about what is *knowable*,
   not a proposal to read it per doorbell.

#### ⊘⊘⊘ What a hostile ring actually buys — and it is LESS than w820 claimed

**[CORRECTED w821 by the owner, and the correction matters because it moves the threat model
off the wrong mechanism onto the right one.]**

w820 argued that a hostile ring *"promotes the attacker's timing hint into a control input over
victim-state processing."* ⊘ **That overstates it.** `[owner]`:

> *"it doesn't have access to userd or ring or `GP_GET`/`GP_PUT`, so it's a harmless check to see
> if there is work and then there isn't. That's just cycles, just like the real GPU costs cycles
> to inspect a doorbell."*

★ **Correct, and the mechanism is worth stating exactly.** A worker that takes a spuriously-rung
token reads the victim's `GP_PUT` and compares it to our cursor. If the victim has not submitted,
`GP_PUT == cursor`, **there is no work, and we do nothing**. The attacker supplies a 32-bit token
and nothing else; every byte we then act on comes from memory only the victim can write. ⇒ Our
cost profile is hardware's: **ring anything, pay a redundant look.**

⇒ **The double-take is therefore a CORRECTNESS boundary, not a security one**, and w820 was wrong
to promote it. Two *legitimate* rings from the victim itself race exactly the same way — the
attacker only makes the window easy to hit. ★ The four-state token word in §2.4 is still
**mandatory**; what changes is *why*. It is owed to the victim's own concurrency, not to an
adversary, and a design that justified it by the adversary would have deleted it the moment the
adversary was ruled out.

⚠ **The one residual, named rather than claimed absent:** the passthrough branch and the managed
branch differ in cost (an inline MMIO write versus two `fetch_or`s), so ring timing is a weak
oracle for *"does a channel exist at this token, and is it passthrough."* ⊘ Both branches are
O(1) with no data-dependent work on victim state, so there is no timing channel into anything the
victim owns. `[owner]` *"barely any meaningful timing of the MMIO trap can be observed."* Agreed;
recorded so the claim is bounded rather than absolute.

#### ★★★ So the threat is not what we parse — it is what an unprivileged write can REACH

`[owner, and this is the sharp version of the whole section]`: the hazard is not that an
unprivileged process makes us look at a victim's channel. It is that an unprivileged write, landing
on a page guest userspace can map, **must not be able to reach the privileged plane or exhaust
anything**. Two rules follow, and they are the real content of §47:

- ⊘ **A degradation path is an amplifier.** Any *"queue full ⇒ scan everything"* fallback lets an
  unprivileged process force work on the attacker's schedule. Deleted (§2.2).
- ⊘⊘⊘ **Any offset in a guest-userspace-mappable page that is NOT the doorbell must return
  immediately, doing nothing.** See §2.2 — this was a live hole in the w820 design and the owner
  found it.

#### ⊘⊘⊘ THE HOPPER+ BREAK — on Hopper and Blackwell the doorbell is written through **BAR1**

**[NEW w821, from an adversarial review. This is the most serious defect found in v3, and it is
an architectural break, not a wording problem.]**

v3 §2 opens *"BAR1/BAR2 are never trapped — ensuring that is a requirement."* ⊘ **On Hopper and
newer that requirement silently discards work submission.**

`[ogkm]` `usermode_api.c:63-98`: for `hClass >= HOPPER_USERMODE_A` the allocation takes
`bBar1Mapping` from the caller, and when set, `pMemDesc = pKernelFifo->pBar1VF` with
`memClassId = NV01_MEMORY_SYSTEM` — a **BAR1** window onto the VF register block
(`kernel_fifo_gh100.c`, `MEMDESC_FLAGS_MAP_SYSCOH_OVER_BAR1`), placed wherever the **guest's** RM
writes a BAR1 PTE for it.

★★★ **`bBar1Mapping` is NOT privilege-gated.** Only `bPriv` is (`!bPrivMapping ||
privLevel >= RS_PRIV_LEVEL_KERNEL`). Any unprivileged client may ask for the BAR1 form.

And both of NVIDIA's own consumers do, with `bPriv = NV_FALSE`:

- **UVM** — `nv_gpu_ops.c:5644`: `.bBar1Mapping = NV_TRUE, .bPriv = NV_FALSE` when
  `isDeviceHopperPlus(device)`.
- **nvidia-push** — `nvidia-push-init.c:968`, carrying NVIDIA's own explanation:
  > *"The BAR1 mapping is used for (faster and more efficient) writes to perform work submission,
  > but can't be used for reads. If we ever want to read from the USERMODE region (e.g., to read
  > PTIMER) then we need a second mapping."*

⇒ Four consequences, and the first is a product failure:

1. ⊘⊘⊘ **Every UVM doorbell — and, on this evidence, every CUDA doorbell — is lost on Hopper+**,
   silently, because it lands on a BAR1 page we deliberately do not trap.
2. ⊘ **§2.1 fact 3 is Ampere/Ada-scoped.** On Hopper+ the guest **kernel** still rings BAR0
   (RM's own CeUtils channel allocates `VOLTA_USERMODE_A` and rings via `GPU_VREG_WR32`) while
   **userspace rings BAR1** — *different pages, different GPAs*.
3. ⊘ **§2.1 fact 1 mis-describes `bPriv`.** It selects the `NV_VIRTUAL_FUNCTION_PRIV` region
   (`pBar1PrivVF`, its own doorbell at `NV_VIRTUAL_FUNCTION_PRIV_DOORBELL 0x2200`), not
   "BAR1-vs-BAR0". `bBar1Mapping` does that. ⇒ There are **three** usermode memdescs on Hopper+
   (`pRegVF`, `pBar1VF`, `pBar1PrivVF`), not one.
4. ⊘ **§0.2 has no row for *which BAR carries the doorbell*.** It is an **A**-axis variable that
   changes **which trap plane exists at all** — the most consequential kind of axis variation, and
   the one §0 was written to catch. It did not.

★★★★★ **And the inversion, which is the genuinely good news and changes what is possible.**
On Hopper+ the privilege separation the owner asked for **exists in hardware, for free**: the
unprivileged doorbell is a *distinct page at a distinct GPA* from the kernel's. ⇒ The literal form
of §47 — *"do not register write traps in the unprivileged page"* — which §2.1 records as
unreachable on Ampere **becomes reachable on Hopper+**, because there the two planes are already
separate. ⊘ The architecture should be built so that separation is *exploited where it exists*
rather than assumed absent everywhere.

#### ✔ RULED, w821 — take (a), and the constraint is lightly lifted

`[owner]` *"nested vGPU is unsupported, so there is only 1 doorbell page possible, and we map the
right one from host userspace to the correct BAR1 MMIO. For that still KVM memslot read-only.
Would then allow BAR1 trapping, but only on that doorbell page, nothing else. Constraint lightly
lifted."*

⚠⚠⚠ **[CORRECTED w821 — I wrote a HYPOTHESIS as an established motive and then leaned on it.]**
The owner offered, and explicitly flagged as *"just a hypothesis, not proven"*, that the BAR1 form
exists so each **vGPU / MIG partition** can own its own doorbell page. I recorded it as
*"the owner's reading of NVIDIA's motive is the load-bearing simplification"* and derived
*"there is exactly one doorbell page"* from it. ⊘ **Both moves were wrong**: the motive is
unproven, and a design decision must not rest on why a vendor did something.

⇒ **The question re-asked without the motive: how many doorbell-bearing apertures can exist?**
That is answerable directly, and the answer is **not one**.

`kfifoConstructUsermodeMemdescs_GH100` (`kernel_fifo_gh100.c:113-132`) builds **two** memdescs in a
loop, then calls the Volta constructor which builds a **third**:

| memdesc | aperture | VF-relative range (GH100) | size | privilege to obtain |
|---|---|---|---|---|
| `pBar1PrivVF` | BAR1 (`ADDR_SYSMEM`, `MAP_SYSCOH_OVER_BAR1`) | `NV_VIRTUAL_FUNCTION_PRIV` `0x00000`–`0x2FFFF` | 192 KiB | ⊘ **kernel** (`bPriv` requires `RS_PRIV_LEVEL_KERNEL`) |
| `pBar1VF` | BAR1 (same flags) | `NV_VIRTUAL_FUNCTION` `0x30000`–`0x3FFFF` | 64 KiB | ★ **none** — `bBar1Mapping` is ungated |
| `pRegVF` | BAR0 (`ADDR_REGMEM`) | the VF register block | 64 KiB | none |

⇒ ★ **Up to three doorbell-bearing apertures per GPU, not one** — and the doorbell lives at `+0x90`
inside `NV_VIRTUAL_FUNCTION`, i.e. in the **first 4 KiB page** of the 64 KiB unprivileged BAR1
mapping.

★★★ **What DOES survive, and it is the fact the design actually needs:** each of those is
**constructed once per GPU, at init** — `pKernelFifo->pBar1VF` is a single memdesc that every
client which asks for the BAR1 form maps. ⇒ The count of *doorbell objects* is bounded and known
without appealing to anyone's motive.

⊘ **Still open, and it is the one that decides "one page":** whether multiple clients mapping that
**same** memdesc land on the **same guest-physical BAR1 page** or get separate BAR1 addresses.
`memdescAddRef` suggests sharing, but that is inference, not a read of the mapping path. **Not
checked.** §10.

★★★★★ **And the reason this no longer matters much — the mechanism was never really resting on
it.** We identify the page **by RM object handle** (below), not by *"there is only one."* ⇒ If it
turns out there are N, we trap N; the classifier keys on `(bar, off)` from a generated descriptor
set either way. **The hypothesis was load-bearing only for the rhetoric, not for the design** —
which is precisely why writing it in as settled fact was worth catching.

**The revised rule, replacing *"BAR1/BAR2 are never trapped"*:**

> **BAR1 is never trapped, with exactly one exception: the single page the guest maps the
> usermode object at. That page is a read-only KVM memslot over the real host doorbell page —
> reads pass through to hardware, writes trap into §2.2. Nothing else in BAR1 is trapped, ever.**

⊘ This is a **lightening**, not a widening: the trapped set grows by **the pages that carry a
doorbell and no others** — one on Ampere/Ada, and on Hopper+ the first page of the 64 KiB
unprivileged BAR1 window (plus the privileged one **only if** we ever serve `bPriv`, which
requires guest kernel privilege and which we have no reason to grant). It stays inside the shape
§2.5 already uses for BAR0's doorbell page.

⚠ **The other 60 KiB of that BAR1 window is guest-userspace-mappable and carries no doorbell.**
⇒ It goes through R2's **middle arm** — trap, return immediately, do nothing — for exactly the
reason §2.2 gives. ⊘ Passing it through writable would hand unprivileged guest userspace a direct
write path to real VF registers.

**How we find that page — and there is a better answer than pattern-matching PTEs.**
`[owner]` observed that the VA manager's PTX-derived diff for BAR1 already carries the mapping, so
the page *is* visible to us. ★ **[PROPOSE]** but the diff should be the **confirmation**, not the
identification:

1. ★ **Primary — the RM side, which is unambiguous.** We serve `GSP_RM_ALLOC` for
   `HOPPER_USERMODE_A` (§2.2 of Part 2), so we already hold the object handle **and** the
   `bBar1Mapping` flag the guest asked for. When that object is subsequently mapped, we know
   which mapping it is **by handle**, with no inference.
2. ◐ **Confirmation — the BAR1 diff.** The PTX walk reports the page; we assert it matches the
   handle-derived answer. ⊘ A disagreement is a **refusal**, not a tiebreak.

⚠ Why not the diff alone: identifying a page by "it looks like the VF register block" is a
**pattern match on guest-chosen data**, and a hostile guest chooses that data. The handle is ours.

**What this adds to the descriptor set:** `doorbell_bar` and `doorbell_off` become generated
per-arch fields, so the trap classifier in §2.2 keys on `(bar, off)` rather than on `off` — which
is why its pseudocode now takes `bar` as a parameter.

⚠ **Still open (§10):** what happens on Hopper+ when the guest allocates the usermode object
**without** `bBar1Mapping` (the BAR0 form is still legal there), and whether both forms can be
live at once in one guest. ⊘ Not yet checked.

★★★ **And the compensation, restated because it changes what is achievable:** on Hopper+ the
guest kernel rings **BAR0** while userspace rings **BAR1**. The two privilege domains are at
**different pages, different GPAs** — so §47's literal form, *"do not register write traps in the
unprivileged page"*, unreachable on Ampere, **is reachable there**. ⇒ The classifier should
exploit that separation where the hardware provides it rather than assume it absent everywhere.

#### ⚠ The tension with the owner's literal instruction, stated rather than resolved silently

`[owner]` *"I would not register write traps in the unprivileged bar0 page."* The strongest form
of that — **map the page writable straight through to hardware, never trap it** — is not
reachable today, for the reason in fact 3: guest RM rings the *same physical register* from the
kernel, so not trapping it also gives up intercepting **kernel** channel doorbells. The scrub is
one of those (§6), and it is the live cross-client leak. So v3 takes the implementable reading:
**trap, but let nothing behind the trap be shared, growable or degradable.**

★ The literal form becomes reachable the day **every channel rung through that page is
passthrough**. That is a real future state, not a fiction, and it is the reason §2.2 is built as
a table rather than a queue: when that day comes, the doorbell trap is deleted rather than
rewritten.

### 2.2 The doorbell plane — a bit table, and the hint ring is DELETED

**[CORRECTED, w820]** The w819 text here said *"push the token on the queue, set this channel's
rung bit; queue full ⇒ set 'inspect all doorbells' flag."* That fallback is exactly the
amplifier above, and the queue is exactly the exhaustible shared structure. **Both are deleted.**
The table alone carries the plane.

```
nvkvm_trap_write(bar, off, val):

  if (bar, off) == chip.doorbell:              # ⊘ generated per die/arch, never a literal (§0.3 r1)
        tok = val & chip.token_mask            # per-die decoded fields (§2.2)
        w = &token[tok]                        # ★ ONE u64 PER TOKEN — see §2.2.1
        r = route_of(w)                        # loaded from that same word
        if r == PASSTHROUGH:
              write the host doorbell INLINE. No queue, no wake, no lock. Return.
        if r == UNKNOWN:
              Return.                          # ⊘⊘ NO bit, NO bump, NO wake — see below
        # MANAGED (translated or emulated):
        prev = fetch_or(w, RUNG)                                        # AcqRel
        if prev already had RUNG:  Return.     # ⊘ already pending: no bump, no wake
        fetch_or(summary[tok >> 6], 1 << (tok & 63))                    # Release
        bump work_seq; maybe write(eventfd)                             # §2.4
        Return.

  elif chip.userspace_mappable(bar, off):      # ★★★ THE GUARD — see below
        Return.                                # do NOTHING. Not a queue push, not a counter.

  else:                                        # PRIVILEGED: reachable only by guest root
        if is_read_register(off): shadow_write(off, val)   # §41 item 4, synchronous
        append to the privileged queue (§2.3)
        bump work_seq; maybe write(eventfd)
        Return.
```

#### ⊘⊘⊘ TWO MORE DEFECTS IN THAT PSEUDOCODE, BOTH MINE, BOTH FOUND AT w821

**(i) `MANAGED` and `UNKNOWN` shared an arm, and that is a §48.2 violation reachable from
unprivileged guest userspace.** The earlier text bumped `work_seq` and wrote the eventfd for an
**unowned** token. ⇒ An unprivileged process ringing a token that names no channel, in a loop,
bumps the sequence forever; §2.4's worker registers only *"if `work_seq == seen`"* and so **never
parks** — N workers at 100 % CPU on that process's schedule. ★ **That is precisely the amplifier
§47 exists to delete, reintroduced two sections later.**
⇒ **The route check must PRECEDE the bump**, `UNKNOWN` must return doing nothing at all, and the
bump must happen only on a genuine **IDLE→RUNG transition** — `fetch_or` already returns the prior
value, so a re-ring of an already-pending token costs one atomic and no wake.

**(ii) The token table had three incompatible layouts across three sections.** §2.2 said 2 bits
packed 32-per-`u64`; §2.3.2 then said *"stamp `enqueue_pos` into that slot"*, which needs a whole
word; and the existing tree's route table is one `AtomicU64` per token. ⊘ They cannot all be true,
and the packed form is **actively broken**: with 32 tokens per word, `CAS RUNG→BUSY` fails whenever
a **neighbour's** bits change, and *"fail ⇒ not ours, move on"* then **skips a live token**.

★ **Resolution: two structures, each doing one job** (§2.2.1).

⊘⊘⊘ **THE GUARD IS NEW AT w821, AND WITHOUT IT THE DESIGN HAD A REMOTE DoS.** `[owner]`:

> *"if the write is in the userspace doorbell page … but not at the doorbell offset, return
> immediately, ignore, do nothing. Otherwise a DoS attack exists by triggering from unprivileged
> userspace the `else` branch and bumping garbage on the privileged ring to try to fill it and
> crash the guest."*

★★★ **Exactly right, and it is the defect §2.1 was written to prevent, sitting in §2.1's own
pseudocode.** The doorbell page is **64 KiB**; the doorbell is **four bytes of it**. Everything
else on that page is guest-userspace-writable and, under the w820 `if/else`, fell through to the
privileged branch — so any unprivileged guest process could push unbounded garbage onto the
plane reserved for guest root and wedge it.

⇒ ★ **The right shape is a three-way classification, not a two-way one**, and the middle arm is
the one the threat model actually needs: *doorbell* / *guest-userspace-reachable but not the
doorbell* / *privileged*. ⚠ And the middle arm must be derived from **which pages RM maps to
non-kernel clients** (§10.7), not from a hand-written offset range — otherwise a future
userspace-mappable page falls into the privileged arm by default, which is the same bug again.

⊘⊘ **This is also why the earlier framing was harmful and not merely imprecise.** w820 spent its
threat model on *"the attacker makes us parse a victim's pushbuffer"* — which the owner has now
shown is a no-op — while the actual reachable attack was one `else` away in the same function.
★ **A threat model aimed at the wrong mechanism does not merely fail to help; it draws attention
away from the arm that was exploitable.**

⊘⊘⊘ **[CORRECTED w821, from an adversarial review — three defects, all mine.]**
**(a)** The earlier pseudocode indexed `table[tok >> 6]` and `summary[tok >> 15]`. Those are
**1-bit-per-token** shifts, while §2.4 specifies **two** bits per token. The corrected shifts are
above, and the earlier `w = table[tok >> 6]` was a **dead load** besides.
**(b)** It wrote the doorbell offset as the literal `0x00BB0090` — violating this document's own
§0.3 rule 1 in the first place it had the chance to.
**(c)** It deferred **every** non-doorbell write with no synchronous arm, which breaks
`THE_CONSTRAINTS.md` §41 item 4. See §2.3.

⇒ **A vCPU now takes NO lock at all on the doorbell path.** The w819 text said "one lock, the
queue's"; with the queue gone there is none.

### 2.2.1 The two structures — state per token, scanning per bit

⊘ One structure cannot do both jobs. A CAS needs a word **owned by one token**; a scan needs
**density**. So:

| structure | shape | job |
|---|---|---|
| **token word** | ★ **one `u64` per token**: `route` (2 b) · `state` (2 b: IDLE/RUNG/BUSY/BUSY_RUNG) · `stamp` (§2.3.2) · `host_token` | every CAS. **No neighbour can ever fail it.** |
| **rung bitmap** | 1 bit per token, + a one-bit-per-`u64` summary | ⊘ **scanning only**, never CAS'd for state |

⇒ Cost: **4 MiB** of token words at 19 bits, **16 MiB** at the 21-bit worst case — plainly
affordable, and **never scanned**. The thing that *is* scanned is the bitmap: **64 KiB** with a
**1 KiB** summary, which is where the original sizing argument was right.

⚠ **The bitmap is a hint about the words, and the word is the truth** — the same discipline as
§2.2's ring deletion. A bit set with an IDLE word is a stale hint costing one look.
★ And the stamp must be **`fetch_max`, not a store**: two vCPUs ringing one token could otherwise
leave the *older* `enqueue_pos` and let the worker run ahead of a register write.

#### Sizing — ⊘⊘⊘ and my first answer was an axis error I made myself

**[CORRECTED w821, before the owner saw it.]** I first wrote *"2¹⁹ tokens ⇒ a 64 KiB table"* from
`ampere/ga100/dev_ctrl.h:26` and presented it as the design constant. ⊘ **That is true of Ampere
and false as a general claim** — precisely the mistake §0 exists to prevent. The doorbell token
layout is **not one encoding**; it is at least three, and they do not even live in the same
header or constant family:

| chip | `VECTOR` | `RUNLIST_ID` | extra decoded bits | header |
|---|---|---|---|---|
| Turing TU102 | `11:0` | `22:16` | — | `turing/tu102/dev_ctrl.h:36` |
| Ampere GA100 | `11:0` | `22:16` | — | `ampere/ga100/dev_ctrl.h:26` |
| **Blackwell GB202** | `11:0` | `22:16` | ⊘ **`RUNLIST_DOORBELL` `30:30`**, set to `ENABLE` by the token generator (`kernel_fifo_gb202.c`) | `blackwell/gb202/dev_vm.h:28` |
| **Blackwell GB100** | `11:0` | `22:16` | ⊘⊘ **`RUNLIST_DOORBELL` `22:22`** — *overlapping the top bit of `RUNLIST_ID`* — plus **`GSP_DOORBELL` `31:31`**, `RSVD 15:12`, `RSVD2 31:23` | `blackwell/gb100/dev_vm.h:622` |

Three separate hazards fall out, and only the first is about size:

1. ⊘ **The token space is not 2¹⁹ everywhere.** Worst case across the table is bits
   `{11:0, 22:16, 30, 31}` = **21 bits ⇒ 2 Mi tokens ⇒ a 256 KiB table**. ⇒ **[PROPOSE]** allocate
   from the **generated field widths of the running die**, and let 256 KiB be the *bound* the
   allocator is checked against — not a constant anywhere in source. A scan of 256 KiB is still
   ~4096 cachelines behind a 4 KiB summary, which is still cheaper than a ring.
2. ⊘ **The constant family renames across the arch boundary.** Ampere and Turing call it
   `NV_CTRL_VF_DOORBELL` in `dev_ctrl.h`; Blackwell calls it `NV_VIRTUAL_FUNCTION_DOORBELL_*` in
   `dev_vm.h`. ⚠ **A generator keyed on the Ampere name finds nothing on Blackwell and silently
   produces no fields** — an empty result that reads exactly like "no variation here". This is the
   `a_capture_derived_table_expires_as_a_vendor_regression` shape, arriving through a *generator*
   rather than a capture.
3. ✔ **GB100's `GSP_DOORBELL` bit at 31 — RESOLVED w821, and it is safe by construction.**
   w821 first flagged this as *"the sharpest open question in this section"*, on the theory that
   an unprivileged write addressing the **firmware** rather than a runlist would understate
   §2.1's threat model. `[owner]` *"well, kernel channels also have doorbells. Same thing — they
   do not queue on the privileged queue, treated like a normal doorbell, also with the fill, and
   reading when there is no work is a no-op."*
   ★ **Right, and the reason is structural rather than a judgement call: we implement no behaviour
   for that bit at all.** Because the table is sized to what the register can *express*, a token
   with bit 31 set indexes a real slot; because no channel is registered there, the worker finds
   no work and releases. ⇒ It degenerates to the same no-op as any other unowned token.
   ⊘ It is **not** a path to `NV_PGSP_QUEUE_HEAD` — that is a different register at `0x110c00`, on
   a page guest userspace cannot map, and it is the *only* thing that kicks the RPC plane.
   ⇒ **Masking to the full expressible space is what makes this safe**, which is the second time
   that decision has paid for itself.

⇒ `TOKEN_MASK` is therefore **per-die, generated**, and masking remains the right operation
rather than validating: it is what the hardware does with undecoded bits, **and** it removes an
error path — no refusal, no counter, nothing to misread. Sizing to what the register can
*express* rather than to what is *legal* is this tree's own lesson,
`[a_bound_on_reads_is_not_a_bound_on_emits]`: the attacker writes the register, not the legal
space.

★ Scan cost is **arithmetic, not yet measured**: a summary of one bit per `u64` is 1 KiB for
Ampere's table and 4 KiB for the 21-bit worst case; the loaded case streams `u64` compares and
skips zero words. Sub-microsecond warm, low single-digit µs cold. **That is cheaper than
maintaining a ring** — which is why the ring is deleted rather than repaired.

⇒ What a hostile guest process can buy is: one `fetch_or` on two bits in a fixed table that was
going to be scanned anyway. **Exactly hardware's cost profile: ring anything, pay a redundant
look.**

⊘ **[CORRECTED w821]** I wrote *"on a per-token cacheline so there is not even a contention
point."* **False on both halves.** At 2 bits per token a cacheline holds **256 tokens**, and one
summary word covers **2048**; and every doorbell — passthrough excepted — also RMWs the single
`worker_vcpu_poll` line that §2.4 itself says ping-pongs at ~10⁷/s. ⇒ There **is** a contention
point, it is `worker_vcpu_poll`, and the honest claim is that contention is bounded by that one
line rather than growing with the number of channels.

### 2.3 The privileged plane — one ordered ring, and per-vCPU rings were WRONG

**[CORRECTED, w820 — this defect was in my proposal, found by an adversarial review.]** I had
proposed per-vCPU register rings with a single-bit owner claim. Per-vCPU rings preserve only
**per-vCPU** order, and the order the device needs is the one the **guest** established, which
spans vCPUs:

```
T1 on vCPU0, holding an RM lock:  write PDB_LO, PDB_HI   → ring0
T1 releases the lock
T2 on vCPU1, takes the lock:      write TRIGGER          → ring1
Worker B drains ring1 first  ⇒  TRIGGER fires against the OLD PDB
```

On hardware this cannot happen: vCPU0's trap returns before T1 releases the lock, and PCIe
posted writes to one function are ordered. This is not exotic — it is **every cross-CPU register
sequence RM orders with its own lock**.

### ⊘⊘⊘ The mechanism, settled at w821 after an adversarial review

`[owner]` proposed *"a queue that has a lock multiple threads may read from"* instead of a
dedicated drainer, on the grounds that *"an additional context switch is more expensive than one
small lock for the vCPU"*, and asked for this to be investigated before deciding. It was.

★ **What the owner got right, and it is most of it:** the two-lock instinct (a tiny push/pop lock
kept separate from a drain-ownership lock) is the correct shape *if* you use locks at all, and the
within-queue ordering argument is sound — one global FIFO whose enqueue point is totally ordered
preserves the guest's cross-vCPU order, because vCPU0's push completes before its trap returns and
the guest's own lock release/acquire supplies the happens-before.

⊘ **What decides it against locks, and it is not a performance argument.**

**1. The cost comparison weighs the wrong two things.** The wake is paid **in both designs** — the
vCPU writes the eventfd either way. What the shared-lock design saves is a context switch **on a
worker core**, never on the vCPU. ⇒ It trades a cost the vCPU never pays against a risk only the
vCPU bears.

**2. Lock-holder preemption puts a host scheduler slice inside an MMIO trap.** `q.lock` is a
userspace lock; userspace cannot disable preemption. A worker holding it for ~50 ns can be
preempted with it held, and a vCPU that then traps waits for that worker to be rescheduled —
**a full CFS slice, milliseconds, on an oversubscribed box.** At a few thousand pops/second the
tail fires every few minutes. ⊘ *"Still microseconds"* is the **mean**; constraint 4 is about the
**tail**, and `worst_trap` is exactly the instrument that has caught this class before.

**3. Under `SCHED_FIFO` vCPUs the tail becomes a deadlock.** A FIFO vCPU spinning on a lock held
by a preempted `SCHED_OTHER` worker on the same core never yields; the worker cannot run; the
guest core is frozen until the RT throttle intervenes. ⚠ Latency-tuned KVM hosts do run vCPUs
`SCHED_FIFO`. A futex instead of a spinlock only converts the deadlock into a polite blocking
wait — which is still a blocking wait on a vCPU.

**4. "Any worker may drain" also means "possibly no worker will drain."** Workers are the threads
that make blocking host calls. If all of them are inside one, the queue is not drained until one
next polls — which makes any *"wait for a slot"* policy **unbounded**.

⇒ **Decision: lock-free on the vCPU, one dedicated drainer.** `[owner]` had already sanctioned the
fallback — *"holding a register drainer is not that bad; we just need the decision that is
race-free and correct and secure, perf is a second issue"* — and this is that decision. ★ It also
takes the vCPU's lock count from **one to zero**, which is strictly better than §41 item 1 requires
rather than merely compliant with it.

### 2.3.1 The design

1. **One bounded lock-free MPSC ring per device**, fixed capacity, **preallocated and prefaulted**.
   ⊘ Never growable: growth means calling the allocator on a vCPU, possibly faulting — blocking
   work, under a lock, on a vCPU, three invariants in one line.
   ⊘ It has its **own** cursor. `work_seq` is a wake sequence and nothing else (§2.4).
   ⊘⊘ **[CORRECTED w821 — the claim must be CONDITIONAL, and `fetch_add` is not.]** I first wrote
   *"the producer's `fetch_add` on the enqueue cursor is the global order."* ⚠ In the standard
   bounded-ring shape the producer then **waits for the slot's sequence stamp** if the consumer has
   not caught up — *the vCPU waiting on a worker, in lock-free costume* — and a producer that
   claims and then bails leaves the consumer **stalled at that slot forever with no diagnostic**.
   ⇒ **Read the cursor, test fullness, take the slot with a CAS.** On full: poison and return,
   having claimed nothing. On CAS failure: retry, looping only against peer producers that each
   complete in one instruction. ★ `THE_CONSTRAINTS.md` §48.2.
   ⚠ This trades wait-freedom for lock-freedom on the claim, and that is the right trade:
   wait-freedom is worthless if the claim can then be stuck waiting on the consumer.
   ★ **The total order still holds** — a successful CAS on a single cursor is as totally ordered as
   a `fetch_add`, which is all §2.3's cross-vCPU argument needs.
2. **One dedicated drainer thread**, spin-then-park: spin briefly after the last item, then block
   on an `EFD_NONBLOCK` eventfd. ⇒ Zero wakes under load, one at idle.
3. **Peek → apply → commit.** The consumer cursor advances only *after* a successful apply, so a
   failed apply retries at the head instead of being re-queued at the tail, and `applied_seq`
   becomes a clean monotonic number other paths can fence against.
4. ⊘⊘ **[CORRECTED w821 — rule 4 as first written contradicts §41 item 4, on this design's own
   central register.]** I wrote *"the drainer NEVER stores to the shadow of a guest-writable
   register."* ⊘ **`NV_VIRTUAL_FUNCTION_PRIV_MMU_INVALIDATE` breaks it immediately**: the guest
   writes `TRIGGER=1` and **spin-reads the same register until it reads 0**. The vCPU's synchronous
   shadow write stores the 1; **only the drainer, after applying the diff, can clear it.** Same
   shape for every boot-FSM register the guest polls.
   ⇒ **The rule is narrower:** the drainer may write the shadow of a register **whose completion it
   owns**, and must not write one the guest also drives. ⊘ And a plain store is wrong for two more
   classes — `LEAF_EN_SET`/`_CLEAR` are **write-only ports** whose readable effect is on a
   *different* register, and `INTR_LEAF` is **write-1-to-clear** and needs `fetch_and`.
   ⇒ ★ **The register descriptor needs a `write_semantics` field** — `plain` / `w1c` /
   `write_only_port(target)` / `trigger(cleared_by_drainer)` — generated from the swref access
   codes (`-W-4R`, `-WEVF`), not hand-classified.
   **Queue everything; shadow every readable register.** Queueing pure-state registers looks wasteful and is what keeps
   `PDB=X, TRIGGER, PDB=Y, TRIGGER` correct — a drainer that read PDB from the shadow when applying
   the first trigger would see `Y`.
   ⚠ And the clobber it prevents: vCPU writes `A` (shadow=`A`, queued), drainer starts applying
   `A`, vCPU writes `B` (shadow=`B`, queued), drainer finishes and writes back `f(A)` — the guest
   now reads a value older than one it wrote. ⇒ Worker-owned bits live in registers the guest does
   not write, or are updated by atomic RMW; a guest write-1-to-clear is `fetch_and`, **never**
   load-store, or the worker's concurrent set is lost against the ISR's clear.
5. ⊘ **Full ⇒ poison, never wait.** Enter a sticky error state, drop further writes, surface a
   guest-visible fault by the normal mechanism — the emulated equivalent of the card falling off
   the bus — and **do not** write the shadow for a poisoned write. ⊘ Silently dropping a single
   write is the wrong shape: it leaves device state that a later, differently-privileged guest
   process inherits. ★ The privilege line is what licenses poisoning rather than blocking: only
   guest root reaches this queue (§2.2's middle arm), so a flood harms only itself.
   ⚠ Size from measurement, and treat sustained occupancy above ~25 % as a defect to investigate,
   not a number to grow.

### 2.3.2 ★★★ The ordering hole across the queue boundary — found at w821, by neither of us

**Order is preserved *within* the queue. Doorbells go around it.** A passthrough doorbell hits
hardware inline on the vCPU; a managed one sets a table bit a *different* worker consumes. Neither
is ordered against a queued register write:

```
1. Guest writes register R that a channel's servicing depends on.   → shadow + queue. Trap returns.
2. Guest rings the doorbell.                                        → hardware NOW, or a worker NOW.
3. The drainer applies R, 30 µs later.
```

On hardware, posted writes to one function are ordered and the device saw R first. Here the
doorbell is serviced against pre-R state. ⚠ **Whether any current register has that dependency is a
per-die question that will be answered by a hang, not by review.**

⇒ **[PROPOSE] The fence is nearly free and uses space we already allocate.** §2.2's token table is
two bits inside a `u64` slot; the doorbell trap also stamps the current `enqueue_pos` into that
slot, and the doorbell worker services it only once `applied_seq >= stamp`. The vCPU pays one
extra atomic load. ⊘ The worker waits — **the vCPU never does**, which keeps the owner's doorbell
invariant intact: *queued now ⇒ eventually scheduled, never blocks.*

⊘ **Passthrough has no worker to wait, so it needs a proof, not a fence.** The obligation is:
*no queued register write can affect how hardware services a passthrough channel.* Plausible — RM
waits for its RPC replies before ringing, and USERD is memory — **but it is an obligation, and §10
carries it as one.**

### 2.4 The wakeup protocol

One word, `worker_vcpu_poll`, and the futex `prepare_to_wait`-then-`schedule` idiom. `[owner]`
supplied the field split; an adversarial review supplied the orderings and the layout.

```
worker_vcpu_poll : u64  =  { work_seq : 56 (HIGH) , workers_polling : 8 (LOW) }
```

**vCPU** — publish the work, then `fetch_add(1 << 8)` (`AcqRel`); if the pre-value's
`workers_polling > 0`, `write(eventfd, 1)`.
**Worker** — `seen = work_seq` (`Acquire`) **before** scanning; scan every source; then CAS to
register as polling **only if `work_seq == seen`** (`AcqRel`); on failure, rescan.

★ Registering only when the sequence is unchanged is what deletes the "who clears the flag, and
when" question that the flag-based version could not answer. Any work published after `seen`
fails the CAS, so a worker cannot sleep on work it did not see.

**Why 56/8 is right, with the number.** ABA needs the counter to wrap *inside one worker's scan*,
not globally — and every vCPU CASes the **same cacheline**, so aggregate bump rate is capped by
ping-pong at roughly 10⁷/s regardless of vCPU count. 32 bits wraps in ~430 s (a descheduled
worker could straddle it); 40 bits in ~30 h; **56 bits in ~228 years.** No spin-loop fallback is
needed.

⊘ **`work_seq` must be the HIGH half.** With it low, the carry at 2⁵⁶ lands in `workers_polling`
— a phantom poller, permanent, making every subsequent vCPU trap pay a `write()` syscall,
silently. High, the carry falls off the top of the `u64`. The benign direction (a worker's
`fetch_add(1)` carrying *into* `work_seq`) only causes a rescan.

⚠ **Cap the worker count at 255 structurally**, asserted at startup. `workers_polling` wrapping
would read **0 while workers are polling** — precisely the lost wakeup the protocol exists to
prevent. Saturating is worse than asserting: it makes the count wrong in a way that still reads
plausible.

⊘ **Orderings are load-bearing and x86 TSO hides their absence.** With `Relaxed` on the vCPU's
bump, the ring/table store may become visible *after* it on ARM: the worker scans empty,
registers, sleeps, and the vCPU already read `workers_polling == 0` so sends no wake. Passes
every bench in this tree; fails on the first Grace host. Rule: **every write to a work source
happens-before the sequence bump, and every `seen` load happens-before the scan.**

⊘ **The eventfd is a sum, not a queue of wakeups.** M writes landing before the first `read()`
collapse to one wakeup under `EPOLLEXCLUSIVE`, leaving N−1 workers asleep with work queued.
⇒ `EFD_SEMAPHORE | EFD_NONBLOCK`, **one shared epoll instance**, `read()` consumes exactly one.
⚠ And a hard invariant: **no handler may wait on another queued item.** If one ever does, that
collapse stops being latency and becomes a permanent hang.

⊘ **A bit that says work exists is not a bit that says you may touch the hardware.** Per token,
two bits, four states:

```
vCPU:    fetch_or(RUNG)
worker:  CAS RUNG → BUSY      (fail ⇒ not ours, move on)
         act
         CAS BUSY → IDLE      (fail ⇒ state is BUSY_RUNG ⇒ CAS BUSY_RUNG → BUSY, act again)
```

Exactly one worker per channel at a time, no re-ring lost, no lock. ★ Per §2.1 this is a
**security** boundary: without it an unprivileged guest process triggers the concurrent walk of
root's channel deliberately.

⊘ **Never clear `RUNG` on "cannot act yet."** A doorbell can be acted on before a register write
that preceded it on the same vCPU (different planes, §2.2 vs §2.3). On hardware the pending bit
persists until the channel is schedulable; ours must too. Clearing it on a refusal is a lost
doorbell — the same class as this tree's `forwarded=0 refused=8` engine-object rows.

⊘⊘ **[CORRECTED w821 — the four states as written cannot express that.]** `CAS RUNG → BUSY`
**clears** `RUNG`. A worker that then finds it cannot act has no transition back. ⇒ Add
**`BUSY → RUNG`** as the *"put it back"* edge, distinct from `BUSY → IDLE`.
⚠ And that creates a livelock the doc must close: an unactionable token is then **found on every
scan**, so a worker treating *"found work"* as *"do not sleep"* spins forever — while the thing
that would make the channel schedulable is a **register write or RPC that also needs a worker**.
⇒ **"Found but unactionable" must count as "no work" for the purpose of deciding to sleep.** The
`work_seq` bump from the register drainer is what wakes it again, and that is exactly the wakeup
the sequence protocol already provides.

⊘⊘⊘ **[NEW w821 — a lost doorbell IS reachable with the summary bitmap, and I never specified its
clearing discipline.]** The interleaving:

```
Worker W: seen = work_seq
W: load table word w  → 0
   vCPU:  fetch_or(table[w], RUNG)            ← work appears
   vCPU:  fetch_or(summary, bit w)
   vCPU:  bump work_seq → S+1
W: CLEAR summary bit w          (it saw the word as 0)
W: rescan, trusting the summary → misses the token
W: CAS register-as-polling with work_seq == S+1 → SUCCEEDS (the bump preceded the rescan)
W: sleeps.   RUNG set, summary clear, no further bump.   ⊘ LOST until an unrelated ring.
```

★ The fix is the standard one and must be **in the API shape**, not a comment: **clear the summary
bit first, then re-read the word** (`Release` on the clear, `Acquire` on the re-read); if the word
is non-zero, set the bit back. ⊘ A summary that is *never* cleared is not an index, and a scan
that always walks the full table makes the summary pointless — so "just don't clear it" is not an
escape.

⚠ **Deregistration is unspecified above and needs to be:** who decrements `workers_polling`, and
when relative to the `read()`. §2.4's Q3-style safety argument assumes deregister-then-scan; that
ordering is the contract, not an implementation detail.

⊘ **[CORRECTED w821] The eventfd paragraph contradicts itself.** It mandates
`EFD_SEMAPHORE | EFD_NONBLOCK` with one shared epoll, and then the instrumentation note describes
workers *parked in a blocking `read()`*. With `EFD_NONBLOCK` no worker ever parks there. ⇒ Keep
the non-blocking form; the instrumentation note applies only to the blocking variant and is
retained as a warning about **which** design it would bite. ⚠ Also: with a **single shared** epoll
instance `EPOLLEXCLUSIVE` is irrelevant — it arbitrates between epoll *instances*. The wakeup
collapse it was cited against is a property of the per-worker-epoll configuration we are **not**
using.

⊘ **The claim is acquired INSIDE the scan, after `seen`, and released before the scan returns.**
Caching "this ring was empty" across a `seen` read loses an item **forever**. This must be
enforced by the API shape, not by a comment, because it is exactly the shape someone optimises.

⚠ **Instrumentation note.** A worker parked in a blocking `read()` counts in `workers_polling`
but is **not** in `epoll_wait`. A census built on the word and one built on epoll state disagree
only under load — the one condition in which anyone consults them.

### 2.5 The doorbell page is READ-ONLY, not emulated

★★★ The doorbell page is mapped into the guest as a **KVM read-only memslot** over the real
host doorbell page. Consequences, all of them deletions:

- Every **read** is hardware, with **no exit and no code** — including the microsecond counter
  *on this page*.
  ⊘⊘ **[CORRECTED w821 — "there is no PTIMER implementation in v3" was overdrawn.]** The
  read-only memslot removes the **usermode window's** timer (`NVC361_TIME_0/1` at page `+0x080`).
  It does **not** remove the **kernel's** clock: RM reads `NV_PTIMER_TIME_0/1` at
  **`0x9400`/`0x9410`** (`kepler/gk104/dev_timer.h:35`) — a *different BAR0 page*, reached with
  ordinary register reads. ⇒ That page needs its own answer, and each option costs something:
  read-trap it (contradicts *"only writes trap"*), shadow it (is emulation, which §8 deletes), or
  memslot it over the host's page (⚠ a page of **side-effect registers**, which the constraints
  forbid mapping wholesale). `THE_CONSTRAINTS.md` already anticipates this — *"possibly with that
  one page still trapped."* **Unresolved; §10.**
- Every **write** traps, and runs §2.2.
- When *we* need to ring a doorbell (passthrough inline, or translated from a worker) we write
  the host page **directly**, bypassing the read-only mapping.

⚠ Emulated channels need no doorbell written at all — there is no hardware counterpart.

★ **[CORRECTED at w819] Passthrough and managed are different costs, and I had them merged.** A
passthrough doorbell is a table lookup and one dword, synchronously, on the vCPU — **microseconds
and a return**. Only translated/emulated doorbells set a bit and wake. Translated and emulated
behave identically from the vCPU's side; only passthrough is inline.

⇒ **This excludes `ioeventfd` for passthrough**: an ioeventfd would buy a cheap exit and then pay
for it with a thread context switch to deliver a dword the vCPU could already have written.

---

## 3. Memory

Guest vidmem is **one host allocation**, and its size is a **command-line parameter of the
hypervisor invocation**, exactly like guest RAM. **[CORRECTED]** v2 sized it by a bisecting
probe and then rebound the advertised framebuffer to whatever the probe won; that is a
measurement standing in for a decision. An operator asking for 12 GiB either gets it or gets a
refusal at startup.

Guest FB offset **is** the offset into that allocation. Guest sysmem is the hypervisor's memfd
reached through a static layout.

---

## 4. Addresses

### 4.1 Mirror, never adopt — and the review's reasons are stronger than mine

**[CORRECTED]** I argued adopt was merely unsound because guest PTEs hold guest-physical
addresses. The review gives three independent defeaters, any one of which is fatal:

1. You **race the host kernel module's own tables and invalidates**.
2. The host driver **exposes no userspace interface** to participate in its locking, so adopt
   needs a **host kernel module** — a deployment bottleneck for the whole product.
3. You need a translation **anyway**, because guest physical ≠ host physical. The only escape
   is a contiguous host range for guest physical, which then fails on **fragmentation** even
   when there is free memory.

⇒ Mirror is also **completely rootless**, which is the security posture the product needs.

### 4.2 The diff

The PTX walk kernel produces a **per-VA-space delta**: a list of VA spaces that changed, and
the entries to apply to each. ⊘ **The previous state lives in vidmem, held by the PTX itself**
— a snapshot of the full PD/PT tree per VA space, keyed by root address, with allocations
freed when no longer referenced. **There is no host-side table of old pages.**

The delta is executed by the VA manager thread as `mmap`/`munmap`.

★ **Batching, and it is the answer to the syscall-cost question.** `[owner]` set the **TLB
defer flag** — which ogkm demonstrably has — on every mapping call but the last. One
invalidate for the whole batch instead of one per page.

### 4.3 A VA space is the OBJECT. The PDB is an attribute of it, per GPU.

⊘⊘⊘ **[CORRECTED TWICE — and ogkm refutes the correction I accepted.]** I first wrote that a
root is *only* a walk seed and never an identity. The review corrected me to
`(PDB base, aperture)`, and I agreed. **ogkm says neither of us was right, and the original
`Option<Pdb>` design was.** Four independent reasons, all cited:

1. **RM's identity is the object plus `vasUniqueId`**, a monotonic atomic assigned at creation
   (`virt_mem_mgr.c:143`). Every runtime lookup is by client handle, never by PDB.
2. **One VA space has N PDBs on N GPUs** — the PDB lives in per-GPU state
   (`gvaspaceGetPageDirBase(pGVAS, pGpu)`, `gpu_vaspace.c:1874`).
3. **A VA space can CHANGE its PDB, and can have NONE.** `SET_PAGE_DIRECTORY` is explicitly
   repeatable for resize/migrate (`nvos.h:3084`); `gvaspaceResize` migrates the root
   (`gpu_vaspace.c:3235`); and the same function reads
   `if (NULL == gvaspaceGetPageDirBase(...)) goto doneGpu;`. Hardware even encodes the absent
   case: `NV_RAMIN_SC_PAGE_DIR_BASE_TARGET_INVALID`.
4. **Nothing stops two VA spaces naming the same PDB** — there is no global PDB→VAS registry;
   RM's only check is local to the target VAS (`gpu_vaspace.c:2855`).

⇒ **`(PDB, aperture)` is a point-in-time HARDWARE FINGERPRINT, not a durable identity.** Use
the VA-space object (its resource key) as identity; carry `Option<(Pdb, Aperture)>` as a
mutable, per-GPU, possibly-absent attribute used to seed a walk and to match hardware state.

★ This retroactively vindicates the existing `Vas::pdb` doc — *"`None` until a declaration
arrives, and a space is nameable, routable and populatable before then"* — which is exactly
what ogkm describes. ⚠ And it means my `Pdb(0)` fix treated a symptom: the durable fix is to
stop using the fingerprint as the key at all.

### 4.4 Where the root actually comes from — it is ours to write

⊘ **Root discovery is not a problem, and the reason is stronger than "we can find it".**
`_gmmuWalkCBUpdatePdb` is a **no-op inside a guest or CPU RM** on GSP parts
(`gmmu_walk.c:599`): *channel instance blocks are written by GSP-RM* — **which is us**. The
guest RPCs the instance block's physical address to GSP (`kernel_channel.c:2626`). We are the
party that programs `NV_RAMIN_PAGE_DIR_BASE_{TARGET,VOL,LO,HI}`, and `TARGET` is the aperture,
written as one unit with the address (`kern_gmmu_gp100.c:123`).

★★★ And `COPY_SERVER_RESERVED_PDES` is **the guest handing us real PDE physical addresses.**
In the GSP split one logical VA space exists as *two* objects: client RM owns all page-table
memory; **server RM — us — gets an exclusive 512 MiB window** (`0x1_0000_0000`, size
`0x2000_0000`) plus the physical addresses, apertures and page shifts of the client's PDE
pages for that window, so the server can map into the same hardware tables without allocating
any (`ctrl90f1.h:261`, `gpu_vaspace.c:4216`).

⊘ **`DMA_SET_PAGE_DIRECTORY` is UVM's call, not a mapping call.** "DMA" is the *device's
DMA/VA-management sub-interface* of `NV01_DEVICE_0` — legacy taxonomy, and every sibling
(`GET_PTE_INFO`, `INVALIDATE_TLB`, `SET_VA_SPACE_SIZE`) is VA-space-wide. What it does is
relocate a VA space's top-level page directory onto **client-supplied** memory; the caller is
UVM at GPU-VA-space registration (`nv_gpu_ops.c:8986`). `UNSET` is its exact inverse.

⊘ **`DUP_OBJECT` aliases, it does not copy.** `vaspaceapiCopyConstruct` is literally
`vaspaceIncRefCnt(pVAS); pVaspaceApi->pVASpace = pVAS;` (`vaspace_api.c:440`). Two handles,
one object — the normal UVM flow, not an anomaly.

---

## 5. Completions and interrupts

**[CORRECTED] Most completion code deletes.**

| kind | what we do |
|---|---|
| **Passthrough** | nothing. We do not inspect the channel and do not care when it finishes. |
| **Translated** | nothing. The semaphore address was already forwarded; the GPU writes it. |
| **Emulated** | forge the completion at the end of our own call — there was no GPU work. |

⇒ The completion-watch list, the 250 ms observer sweep and the CPU-executor completion writer
all go.

**What remains is the armed interrupt — and ogkm settles its shape.**

★★★ **Registration is PER ENGINE, not per channel and not per semaphore.**
`NV01_EVENT_NONSTALL_INTR` *requires* its notifier to be a **Subdevice**; the notify index maps
to an engine (`NV2080_NOTIFIERS_CE0..n`, GR, HOST) and lands in
`pGpu->engineNonstallIntrEventNotifications[rmEngineId]` (`event_notification.c:688`). The
channel class `NVA06F` has **no completion event at all** — only error/RC notification. So the
interrupt is a **broadcast wake to every waiter on that engine**, carrying no channel and no
semaphore identity.

★★★ **The race is the WAITER'S burden, and it is handled three independent ways.** RM's own
semaphore-surface registration re-reads the value under a spinlock and returns
`NV_ERR_ALREADY_SIGNALLED` — with the race written out in a comment (`sem_surf.c:1405`);
callers loop on it (`nvidia-drm-fence.c:991`); and the OS-event wait path **never trusts the
event**, looping on the notifier word in memory and using the event only to decide *when to
re-check* (`nvidia-push.c:751`).

⇒ **`[owner's hypothesis, confirmed]`** we only need to fire on **genuinely new completed
work** — an advance of the engine. We do **not** have to fire retroactively for work already
complete, because the guest re-checks after arming. ⚠ But the converse is binding: once armed,
a later release **must** produce an interrupt. There is no level-triggered delivery to fall
back on.

⊘ **And the interrupt is requested in the PUSHBUFFER**, not by a control: `NV906F_NON_STALL_
INTERRUPT` pushed after the release (`nvidia-push.c:967`), or CE `LAUNCH_DMA ..._INTERRUPT_
TYPE_NON_BLOCKING`, which RM sets **only when a completion callback was requested**
(`channel_utils.c:657`; default `_NONE`). ⚠ There is **no `AWAKEN_ENABLE` field** in the
headers shipped here — an earlier note in this tree citing one should be re-read.

★ **The channels we most care about POLL anyway.** UVM never uses interrupts for channel
completion — it spins (`uvm_channel.c:2158`). The CeUtils scrubber registers a non-stall event
only for its *asynchronous callback* API; every blocking wait is
`while (READ_CHANNEL_PAYLOAD_SEMA(..) < target) { ... }` (`channel_utils.c:343`), and when it
holds the GPU lock it services the ISR **inline** rather than sleeping.

---

## 6. The scrub

★ **There IS a dedicated scrub channel, and a second one beside it.** `OBJMEMSCRUB` owns a
`CeUtils` channel per heap, kind `CE_SCRUBBER_CHANNEL` or `FAST_SCRUBBER_CHANNEL` (SEC2's
`SWL_SCRUBBER_CHANNEL` under Confidential Compute) — `mem_scrub.c:171`, `ce_utils.c:246`. A
**separate** `CeUtils` instance serves MemoryManager's general internal copies
(`mem_mgr.c:4114`), so scrubbing and other RM-internal CE work do not share a channel.

⇒ That makes the scrub a clean, named, **translated** channel. The guest's own RM scrubs before reuse and orders it correctly; our
only job is to let the work reach the GPU instead of forging its completion. `[measured w813]`
today the kernel tokens are `forwarded=0` — 140 doorbells rung, none sent — which is the
cross-client leak.

---

## 7. Portability — derive, do not hardcode

`[owner]` in priority order:

1. Derive from **host userspace calls** wherever possible.
2. **Computed** values are allowed.
3. Generate Rust from the **C headers** of ogkm and friends — dies, families, driver versions.
4. Read **nvkvm-pv** for how it already solved this.

⊘ **Do not parse C with regex or sed.** Use a real parser/compiler front end. `[owner]` *"That
will age better."* ★ This tree's own history agrees: a capture-derived table expired as a
vendor regression, and 11 of 56 captured rows were empty and every empty one that was checked
against hardware was **contradicted**.

The surviving hand-written surface should be a small set of per-architecture-series constants.

---

## 8. What is deleted

Isolates and the whole IPC plane · the sandbox plane · `Proc` · the address table · joins
(all four generations) · VA→phys translation on the submission path · the operand gate ·
publication epochs, the dirty gate, sweep-skip · the CPU CE executor · the completion watch ·
the *usermode-page* PTIMER emulation (⊘ **not** the kernel `NV_PTIMER` page at `0x9400` — see
§2.5) · the framebuffer probe/rebind · **every on/off flag for things that no
longer have two sides** `[owner]`.

★ **[w820] Three more, and they were in the v3 proposal itself — not in the old tree:**

- **The doorbell hint queue.** Superseded by the 64 KiB bit table (§2.2). Deleting it deletes
  ring overflow, the multi-consumer problem for doorbells, and an unprivileged guest process's
  ability to exhaust anything.
- **"Inspect all doorbells" / any FULL_REFRESH-shaped fallback.** There is no refresh because
  there is no queue to fall back *from* — there is only the scan, which is the normal mode.
  `[owner, 2026-09-20]` *"guest userspace can exist without something like FULL_REFRESH."*
- **Per-vCPU register rings and their owner-claim bit.** Refuted in §2.3: they preserve
  per-vCPU order where the device needs the guest's cross-vCPU order.

⊘ Deleting a mechanism that only ever existed in a design document costs nothing and is the
cheapest deletion available. ★ Both doorbell deletions were **licensed by a threat model**, not
by a benchmark — which is the only reason they were found before the code existed.

⊘ **[CORRECTED] What to keep is narrower than I said.** I proposed keeping "the instruments".
The review is right that some instrument *prose* is actively misleading — three stale "The
default" comments in one day, one of which made a full survey report that the single store does
not run. ⇒ Keep the **measurements** and the **gates**; audit the **prose**, and delete what
can no longer be checked. A comment asserting a default will rot; a comment asserting a dated
measurement will not.

---

## 9. Rewrite, not refactor

`[owner asked for an honest view]` **Rewrite, in a fresh crate set, with the old tree as a
read-only reference.**

The deletions above are not edits inside the existing files — they are the removal of the
organising ideas those files are built around (per-proc isolation, an address table, a
publication pass). Deleting them incrementally means running two architectures in one file set
for the duration, which is how the stale defaults and contradictory tests got there in the
first place.

⚠ The one non-negotiable: **the rewrite must be graded continuously against the existing
suite** — the 30-arm guest suite, the bare-metal baseline, and the hardware gate
(`forwarded=` per token). A rewrite that cannot be graded is a rewrite that cannot be landed.

⊘ `[measured 2026-09-20]` `crates/*/src` is **144,175 lines of code** and 125,503 lines of
comment. A 50k target is a ~3x reduction of code, and the four crates carrying the deletions
are `kayfabe-isolate-host` (46,895), `kayfabe-abi` (43,588), `kayfabe-qemu-raw` (37,057) and
`kayfabe-device` (27,496). `kayfabe-abi` should shrink by being **generated** rather than
deleted.

---

## 10. Still open — and what the owner ruled at w821

⊘ **Numbering was out of order and one item appeared on both the resolved and open lists.**
Rebuilt.

### 10.1 Ruled at w821 — closed, with where each ruling landed

| # | question | ruling | lands in |
|---|---|---|---|
| R1 | Does a hostile doorbell give an attacker a control input? | ⊘ **No.** It has no access to USERD, the ring or `GP_PUT`; it costs cycles, as on hardware. The double-take is a **correctness** boundary owed to the victim's own concurrency, not a security one | §2.1 |
| R2 | Non-doorbell offsets in a guest-userspace-mappable page | ⊘⊘⊘ **Return immediately, do nothing.** Without this, unprivileged userspace floods the privileged plane — a live DoS in the w820 design | §2.2 |
| R3 | The Hopper+ BAR1 doorbell | ✔ **Trap the doorbell-bearing page(s) and nothing else**, read-only memslot, `doorbell_bar`/`doorbell_off` generated per arch. ⚠ **[AMENDED w821]** the ruling was first recorded as *"exactly one doorbell page exists"*, derived from an **unproven hypothesis** about NVIDIA's motive. There are up to **three** doorbell-bearing apertures per GPU; the ruling stands because identification is **by RM handle**, not by count | §2.1 |
| R4 | GB100's `GSP_DOORBELL` bit 31 | ✔ **Safe by construction** — we implement no behaviour for it, so it degenerates to an unowned token and a no-op. Kernel channels have doorbells too and are treated identically | §2.2 |
| R5 | Per-arch constant-family renames | ✔ **Code it per giant GPU family** — the rename boundary is the family boundary, so it is a bounded, enumerable set | §0.3, §2.2 |
| R6 | Fault delivery | ✔ **We do fault DELIVERY; we guarantee no fault RECOVERY.** Recovery is unreachable from guest userspace, so we never need to reverse-engineer it — ogkm's recovery cases are nameable and each gets a named workaround. UVM *managed* memory is later and is **simulatable by forcing DMA memory**, which never changes aperture | §2.1 of Part 2, §10.3 |
| R7 | BQL | ✔ **Never on a vCPU.** Permitted only in slow paths that are not our MMIO traps — memslot moves, installation time | §1 |
| R8 | Locks on a vCPU | ✔ **At most one, microseconds, for MMIO only.** Slow paths (driver setup) may take more, including BQL | §2.3, §41 |
| R9 | Batching the VA diff | ✔ **Confirmed as intended**: set the TLB-defer flag on every mapping call but the last, minimising host-side invalidate barriers within one refresh. ⚠ **A refresh on the host must still happen last** — the defer batches the barrier, it does not remove it | §4.2 |
| R10 | Turing | ✔ **Must be supported.** It is the declared floor and it is *not* the easy end — it needs the `GP10X` format family, which our one built format (`GA10X`) is a superset of by one match arm ⇒ **additive** | §0 |
| R11 | Multi-GPU / non-GA10x chip tables | ✔ **Rewrite the chip table to be derivable** rather than special-casing. Judged fixable | §7 |
| R12 | The register queue, after investigation | ✔ **Keep the dedicated drainer thread.** `[owner]` *"fable argued hard to keep it and the arguments are correct. The MMIO trap must be extremely fast, the rest has much more room in the millisecond."* ⇒ Generalised as **`THE_CONSTRAINTS.md` §48** — the latency budget is asymmetric, and a thread spent to keep the trap lock-free is a good trade, not an extravagance | §2.3, §48 |

### 10.2 Genuinely open

1. **The interrupt race, narrowed.** §5 settles *whose burden* (the **waiter's**, three
   independent ways). What is open: how to honour *"once armed, a later release must produce an
   interrupt"* when registration is **per engine** and carries no channel or semaphore identity —
   and when `NVC36F_NON_STALL_INTERRUPT`, the method that *requests* it, is **not recognised
   anywhere in the tree**.
2. ✔ **The privileged queue — SETTLED at w821 (§2.3), investigated as the owner asked.**
   Lock-free bounded MPSC + one dedicated drainer, spin-then-park; overflow **poisons** rather than
   waits. ⇒ The vCPU's lock count goes from one to **zero**.
   ★ The deciding argument is not performance: a shared lock puts a **host scheduler slice inside
   an MMIO trap** via lock-holder preemption (milliseconds in the tail, a deadlock under
   `SCHED_FIFO` vCPUs), and *"any worker may drain"* also means *"possibly no worker will drain"*,
   which makes any wait-for-a-slot policy unbounded.
   ⚠ **Still open under it:** the ring's capacity must come from a **measurement**, not a guess.
3. ★ **Where the walk's root comes from.** `[owner]` this should be analysable from ogkm — the
   call must be used somewhere, and if it is only UVM-managed memory then it is the wrong thing to
   be looking at. ⇒ **And the owner supplies the design answer that makes it actionable:** the
   walk already tells us whether a leaf is **sysmem or vidmem physical**, and *that* decides the
   mechanism — an **RM allocation at a VA** in the VA-manager thread, or a **DMA-map ioctl**.
4. **The crate model** — to be redrawn against §8.
5. **The scan-cost figure in §2.2 is arithmetic, not a measurement.**
6. **Enumerate every BAR0/BAR1 range RM maps to a non-kernel client**, generated rather than
   asserted. ★ R2 makes this load-bearing: the *middle* arm of the trap classifier is defined by
   this set, and a page missing from it falls into the **privileged** arm by default — which is
   R2's bug again, arriving by omission.
7. **The kernel `NV_PTIMER` page at `0x9400`** — §2.5 covers the usermode window only.
8. **A vIOMMU in the guest hands us IOVAs, not GPAs.** Unhandled; the real content of axis `K`.
9. **Windows guest** — `[owner]` read ogkm's Windows-specific code and lawfully published
   material on NVIDIA's Windows behaviour. ⊘ In progress; nothing claimed yet.
13. ⊘⊘ **[NEW w821] The passthrough-doorbell ordering obligation.** §2.3.2 fences *managed*
   doorbells against queued register writes with an `applied_seq` stamp. A **passthrough** doorbell
   has no worker to wait, so it needs a **proof** that no queued register write can affect how
   hardware services a passthrough channel. Plausible (RM awaits its RPC replies before ringing;
   USERD is memory) — ⊘ but unproven, and the failure mode is a hang, not a diagnostic.
14. ★ **[NEW w821] The privileged ring's capacity**, from measured peak occupancy under the
   constraint suite. Sustained occupancy above ~25 % is a defect to investigate, not a number to
   raise.
10. **Hopper+ with the BAR0 form of the usermode object** — still legal there. Whether both forms
   can be live at once in one guest is unchecked (R3).
11. ⊘ **Do multiple clients mapping the SAME `pBar1VF` memdesc share one guest-physical BAR1
   page, or get separate BAR1 addresses?** `memdescAddRef` suggests sharing, but that is inference
   and the mapping path was not read. ★ This is what actually decides whether *"one page"* is true
   on Hopper+; the handle-based identification in R3 is correct either way, but the **size of the
   trapped set** depends on it.
12. ⚠ **A standing hygiene item, from w821:** an owner **hypothesis** was written into this
   document as an established vendor motive and then used to justify a design simplification.
   ⇒ **A claim's epistemic status is part of its citation**, exactly as its date is. Hypotheses get
   marked, and a design decision may not rest on one.

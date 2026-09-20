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
| **K** | **guest kernel** version | ⊘ all major versions | never our concern directly — but it sets what the guest driver is *built against*, so it moves `Dg`'s struct layouts |
| **Dg** | **guest driver** version | ⊘ all major versions | **generated** from that version's headers; a per-version **profile entry**, never `if version ==` |
| **Dh** | **host driver** version | ⊘ all major versions, **decoupled from `Dg`** | we **author** every host call; our host-verb signatures do not accept a guest flag word |
| **A** | **GPU architecture** | ★ Turing and newer | a **format family** descriptor (four families span Turing→Blackwell) |
| **die** | **GPU die** within an arch | ⊘ any die | derived per die, **maintained per family**; a new die is a descriptor, not a code path |
| **V** | **VMM** | ◐ **the one axis where a version floor is legitimate** — QEMU, Cloud Hypervisor | the core is VMM-agnostic; the VMM shim is the only place that knows |
| **OS** | **guest OS** | Linux **and** Windows | ⚠ the axis with the least coverage; everything measured in this campaign used a Linux guest |

⊘ **`Dg` and `Dh` are TWO axes, not one.** The operator chooses the guest driver; we do not.
A design that assumes they match is a defect. ★ This is the structural reason for
`author_host_flags_never_forward_them`: forwarding a guest-supplied flag word into a host RM
call is not merely risky, it is **an implicit assertion that the two versions agree**.

⊘ **Host OS is Linux only** — the one scope relaxation, taken deliberately.

### 0.1 The axis band for (Dg, Dh) is a DIAGONAL, not a product

`[owner ruling, 2026-08-09]` We owe NVIDIA vGPU's own interoperability policy and no more:
exact match; same major branch different minor; and **n−1** (a newer host supports the previous
major or LTS branch). ⇒ A bounded window, not every pair.

★ What that **obliges us to build**: we must know *both* versions and be able to say
*"this pair is outside the band"* — **refused by name**. ⊘ A compatibility policy that cannot
state an out-of-band pair is not a policy, it is a hope, and it decays into the
`0x56 is the forgiven status` trap where a wrong configuration simply runs on.

### 0.2 ★★★ The axes are not evenly distributed across the four planes

This is the useful part, and the surveys in Part 2 measured it rather than assuming it:

| plane | dominant axis | evidence |
|---|---|---|
| **GSP RPC function numbers** | ★ **stable** across `Dg` in the measured window | the `NV_VGPU_MSG_FUNCTION_*` enum is **byte-identical between 580.159.04 and 610.43.02** for every id we use; 610 only *appends* (`rpc_global_enums.h`) |
| **GSP RPC message framing** | ⊘ **`Dg`, and it BREAKS** | the per-element header is **48 bytes** at 580 (`authTag`/`aad`/`checkSum@32`/`seqNum@36`/`elemCount@40`) and **16 bytes** at 610 (`mctpHeader`/`nvdmHeader`/`checkSum@8`/`seqNum@12`) — `message_queue_priv.h:43-51` vs `:52-67` |
| **RM control command numbers** | `Dg` (additive), **struct layouts break** | FINN-generated; the *number* is stable, the *parameter struct* is versioned |
| **BAR0 register offsets** | **A** and **die** | `NV_PGSP_QUEUE_HEAD = 0x110c00` from `ampere/ga102/dev_gsp.h`; the doorbell is `0x30090` on Turing, Ampere **and** Blackwell |
| **Doorbell token encoding** | ⊘ **die**, within one arch | `NV_CTRL_VF_DOORBELL` field widths are per-arch swref, and GB202 sets bit 30 where Ampere does not |
| **Channel / engine class ids** | **A** | `VOLTA_CHANNEL_GPFIFO_A 0xC36F` → `AMPERE_ 0xC56F` → `HOPPER_ 0xC86F` → `BLACKWELL_ 0xC96F` |
| **Pushbuffer method encoding** | ★ **remarkably stable** | `NVC56F_DMA_SEC_OP`/`METHOD_ADDRESS`/`_SUBCHANNEL`/`_COUNT` is bit-identical to `NVC36F_*`; the GPFIFO entry differs only in `PRIV` (dropped at C56F) and `INVAL_SCOPE` (added) |
| **Page-table format** | ⊘ **A**, four families | Turing binds `kgmmuFmtInitLevels_GP10X`; our one built format is `_GA10X` |

⇒ ★★★ **The two planes that break hardest are the two the guest drives most**: RPC framing
(`Dg`) and page-table format (`A`). Both are already modelled as **descriptors** rather than
code paths — `ElementLayout { hdr_size, checksum_off, seqnum_off, elem_count_off, TransportHdr }`
and `GmmuFmt` — and that is the shape every other axis-carrying fact should take.

★ **The 580/610 element header is the worked example to argue from.** It is a structure whose
*size and every field offset* changed between two supported guest driver versions, it was
absorbed as data rather than as a branch, and it proves the discipline is implementable rather
than aspirational.

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
| **worker** | **N, capped at 255** | everything a trap deferred: managed doorbells, GSP RPCs, TLB invalidates |
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
   by `ioremap` of that same base. ⇒ Both mappings land in **one KVM memslot at one GPA**, and
   memslots are keyed by GPA. The trap **cannot distinguish kernel from userspace**. The ring
   cannot be authorized at the trap. Not with more checks. Ever.

#### Why hardware survives this and a naive port does not

On hardware a ring means *"channel (runlist, chid) may have work"*, and the PBDMA then reads
that channel's **USERD `GP_PUT`** — memory only the owner can write. A hostile ring buys a
redundant fetch of work the victim already published. The attacker contributes **zero data**,
only a timing hint. NVIDIA evidently accepted that.

Our handler does not re-read a pointer. For a **translated** channel it parses and translates
the victim's pushbuffer; for an **emulated** channel it executes a function on the victim's
behalf. That promotes the attacker's timing hint into a control input over victim-state
processing, and it converts two things that read as engineering into security defects:

- ⊘ **Concurrent double-take becomes a triggerable primitive.** Ring root's token in a tight
  loop and get two workers walking root's GPFIFO concurrently against one cursor: root's work
  submitted twice, or its cursor walked backwards into a PBDMA ring-wrap. The four-state token
  word in §2.4 is therefore **the boundary**, not a latency nicety.
- ⊘ **A degradation path becomes an amplifier.** Any "queue full ⇒ scan everything" fallback
  lets an unprivileged process force the VMM to walk **everyone's** channels on the attacker's
  schedule.

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
nvkvm_trap_bar0_write(off, val):

  if off == DOORBELL (0x00BB0090):            # VF aperture base + NV_VIRTUAL_FUNCTION_DOORBELL 0x30090
        tok = val & TOKEN_MASK                # mask to the two decoded fields — see below
        w   = table[tok >> 6]
        if PASSTHROUGH(tok):
              write the host doorbell INLINE. No queue, no wake, no lock. Return.
        else:                                 # MANAGED (translated or emulated), or UNKNOWN
              fetch_or(table[tok >> 6], RUNG_BIT(tok))     # AcqRel
              fetch_or(summary[tok >> 15], 1 << ...)       # Release
              bump work_seq; maybe write(eventfd)          # §2.4
              Return.
  else:
        stamp and publish onto the PRIVILEGED ring (§2.3). Return.
```

⇒ **A vCPU now takes NO lock at all on the doorbell path.** The w819 text said "one lock, the
queue's"; with the queue gone there is none.

**Sizing — the number, from the published register, not from the legal space.**
`ampere/ga100/dev_ctrl.h:26`:

```
NV_CTRL_VF_DOORBELL_VECTOR       11:0   →  12 bits  →  4096 channels
NV_CTRL_VF_DOORBELL_RUNLIST_ID   22:16  →   7 bits  →   128 runlists
```

19 decoded bits ⇒ **2¹⁹ = 524 288 tokens ⇒ a 64 KiB bit table**, plus a one-bit-per-`u64`
summary layer = a **1 KiB hot index** over it. Bits 15:12 and 31:23 are undecoded, so `TOKEN_MASK`
keeps only the two fields.

★ Masking rather than validating is deliberate twice over: it is what the hardware does with
undecoded bits, **and** it removes an error path — no refusal, no counter, nothing to misread.
Sizing to what the register can *express* rather than to what is *legal* is this tree's own
lesson, `[a_bound_on_reads_is_not_a_bound_on_emits]`: the attacker writes the register, not the
legal space.

★ Scan cost is **arithmetic, not yet measured**: 1 KiB of summary is ~16 cachelines; the loaded
case streams 8192 `u64` compares over 64 KiB, skipping zero words. Sub-microsecond warm, low
single-digit µs cold. **That is cheaper than maintaining a ring** — which is why the ring is
deleted rather than repaired.

⇒ What a hostile guest process can now buy is: one `fetch_or` on a bit in a fixed table that was
going to be scanned anyway, on a **per-token cacheline** so there is not even a contention point.
**Exactly hardware's cost profile: ring anything, pay a redundant look.**

⚠ Per-die derivation, not a copied constant: those widths come from the Ampere-family `ga100`
swref header. Per §7 they must be **generated from the headers**, and the table sized from the
generated widths — `tu102` agrees today, `gb202`/`gb100` put the doorbell at the same `0x30090`
but their field widths must be read, not assumed.

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

⇒ **One MPSC register ring per device, its slot reserved by the same atomic that bumps
`work_seq`** (§2.4). The sequence is taken *inside* the trap, before it returns, so it already
embeds the guest's cross-vCPU happens-before; the ring is globally ordered by construction and
no separate stamp is needed. The vCPU pays one atomic instead of two, and the per-vCPU claim bit
disappears. A drainer that reaches a reserved-but-unpublished slot waits — bounded by one trap,
microseconds, holding nothing.

⊘ **This ring may not overflow, and that asymmetry is the whole point of §2.1.** The doorbell
table is *state* and a lost hint is recoverable by a scan; a register write is a *stream* and the
stream **is** the truth — there is nothing to rescan and a dropped write is unrecoverable. So it
must be sized so it cannot fill, and the bound comes from ogkm limiting in-flight entries.
⚠ **That bound is asserted, not yet verified** — it belongs in §10, because if it is wrong the
failure is a guest that wedges with no diagnostic. Its saving grace is the privilege line: only
guest root can reach it, and a guest root that overflows it harms only itself.

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

⊘ **The claim is acquired INSIDE the scan, after `seen`, and released before the scan returns.**
Caching "this ring was empty" across a `seen` read loses an item **forever**. This must be
enforced by the API shape, not by a comment, because it is exactly the shape someone optimises.

⚠ **Instrumentation note.** A worker parked in a blocking `read()` counts in `workers_polling`
but is **not** in `epoll_wait`. A census built on the word and one built on epoll state disagree
only under load — the one condition in which anyone consults them.

### 2.5 The doorbell page is READ-ONLY, not emulated

★★★ The doorbell page is mapped into the guest as a **KVM read-only memslot** over the real
host doorbell page. Consequences, all of them deletions:

- Every **read** is hardware, with **no exit and no code** — including the microsecond counter.
  ⊘ **There is no PTIMER implementation in v3.** The register emulation, the refusal of guest
  writes to it, the counter-page install — all gone.
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
PTIMER emulation · the framebuffer probe/rebind · **every on/off flag for things that no
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

## 10. Still open

**[w820] Resolved since w819**, recorded here so the list does not read as unchanged:
the doorbell/register queue split and its overflow semantics (§2.1–2.3); the worker wakeup
protocol, its bit widths, its layout and its memory orderings (§2.4); the doorbell double-take,
now a named security boundary rather than a race (§2.4).

Still genuinely open:

1. **The interrupt race** (§5) — per-channel or per-semaphore, and whose burden.
2. **The synchronous-verb list** — which RM controls does the guest read the reply to
   immediately? Until that list exists, "no RM verb on a vCPU" cannot be judged.
   ⚠ **Owner is waiting on this one**; it blocks judging the trap path end to end.
3. **Fault delivery** — needed by managed memory *and* by letting the GPU fault. `[owner]` if
   the fault structures are shared with userspace we can map them through for passthrough; a
   translated fault needs its address translated on the way back.
4. **The crate model** — to be redrawn against §8.
5. ★ **[NEW, w820] The privileged ring's non-overflow bound is ASSERTED, not verified.**
   §2.3 needs it sized so it cannot fill, on the argument that ogkm limits in-flight entries.
   That bound must be **read out of ogkm and turned into a startup assertion**, because if it
   is wrong the failure is a guest that wedges with no diagnostic.
6. ★ **[NEW, w820] The scan-cost figure in §2.2 is arithmetic, not a measurement.** Sub-µs warm
   is the claim the ring-deletion rests on; it needs a microbenchmark on the bench box before it
   is cited as fact.
7. ★ **[NEW, w820] Enumerate every BAR0 range RM maps to a NON-kernel client.** §2.1 treats the
   usermode window as the unprivileged surface, which is true of ogkm today — but that is **RM
   policy, not an invariant**. The right form is a generated assertion over the objects that
   hand out `ADDR_REGMEM`, not a claim that there is only one. `[owner]` *"same for any other
   page if they exist."*

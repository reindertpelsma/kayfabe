# kayfabe — the design

**STATUS: LIVE, 2026-09-20 (w821).** This states the design as decided. It is written to be
argued with from scratch: no correction history, no record of what earlier drafts said. That
history is in git, and in the superseded documents this one replaces.

---

## 1. What this is

A guest runs a **stock, unpatched NVIDIA driver**. It sees an emulated GPU and a GSP firmware
processor that does not exist. We are that firmware. The guest's real compute reaches a real host
GPU, driven by an **unprivileged** host process.

Two properties make it worth building rather than forwarding ioctls like `nvproxy` or WSL2:

- ★ **The guest driver is unmodified and unaware.** No guest kernel module, no paravirtual ABI, no
  host-side driver patch. The cut line is **below** D3DKMT and below `/dev/nvidia*` — where no
  shipping product cuts.
- ★★★ **Modelling the device forces us to know what things *mean*.** An allowlist filters *what*
  is called; we must know *why*. That is what lets us refuse a flag whose hazard lives in RM's
  interpretation of a bit — including one NVIDIA's own source labels *"This is a bug"*.

**Scope.** GSP-capable parts, **Turing and newer**, because below Turing there is no GSP to be.
Linux and Windows guests; Linux host. QEMU first, VMM-agnostic core. Multi-GPU in the data model
from the start, not in the first milestone.

---

## 2. What varies — the seven axes

Nearly nothing here may be a literal in source, because seven things move underneath it.

| tag | axis | posture | absorbed by |
|---|---|---|---|
| **K** | guest kernel | all major versions | ⚠ not struct layouts — **DMA addressing**: a vIOMMU guest hands us IOVAs, not GPAs |
| **Dg** | guest driver version | all major versions | **generated** per-version descriptors, never `if version ==` |
| **Dh** | host driver version | all major, ⊘ **decoupled from `Dg`** | we **author** every host call; our host verbs take no guest flag word |
| **A** | GPU architecture | Turing+ | a **format-family** descriptor — four families span Turing→Blackwell |
| **die** | die within an arch | any | derived per die, **maintained per family** |
| **V** | VMM | ◐ a version floor is legitimate **here only** | the core is VMM-agnostic; one shim knows QEMU |
| **OS** | guest OS | Linux **and** Windows | detect, never assume |

**Three rules follow.** ⊘ No literal in a comparison. ⊘ No C parsed with regex — generate from
headers with a real parser, and from `rnndb`'s XML where only nouveau has the data. ⊘ No
guest-supplied flag word forwarded into a host call; that is a `Dg`↔`Dh` coupling in disguise.

★ **The worked example that proves the descriptor approach:** the GSP message-element header is
**48 bytes** at driver 580 (`authTag`/`aad`/`checkSum@32`/`seqNum@36`/`elemCount@40`) and **16
bytes** at 610 (`mctpHeader`/`nvdmHeader`/`checkSum@8`/`seqNum@12`). Size and every offset moved,
and a field disappeared. It is absorbed as data. ⚠ And it marks the limit: a version break can also
move **derivation** and **validation**, which a layout descriptor cannot hold.

⚠ **Axes interact.** WPR2 moved on Blackwell to `0x0088A824` *and still answers the legacy
`0x1FA824` for 580-series guests* — the **die** moved it, the **driver version** decides which
address is in play. A per-die table alone answers the wrong address.

---

## 3. The shape

**One process:** the VMM with the kayfabe core linked in. No isolate children, no IPC, no sandbox
plane of our own — sandbox the whole hypervisor process instead.

| thread | count | job |
|---|---|---|
| **vCPU** | guest's | traps only. Never blocks, never waits on us |
| **worker** | N ≤ 255 | the doorbell plane: scan, claim a token, run the channel's work |
| **register drainer** | 1 | drains the privileged ring **in order**. One thread, because an ordered ring drained by many is not ordered |
| **VA manager** | 1 | **all** `mmap`/`munmap` — page-table diffs, BAR windows, channel mappings |

★ The VA manager is a thread rather than a lock so that *"who may map"* is a structural answer.
★ Workers are capped at **255** structurally: the poller count is 8 bits, and a wrap would read
zero while workers are polling — the exact lost wakeup the protocol exists to prevent.

**The latency budget is asymmetric, and it decides most design questions.** Inside an MMIO trap:
microseconds, and **the tail is the statistic**. Everywhere behind it: milliseconds. ⇒ Spending a
thread to keep the trap lock-free is a good trade; moving work into the trap to save a context
switch behind it is a defect however good its average looks.

⚠ **The VMM must be patched for this to be true, and the patch is six lines.** QEMU takes its
global lock unconditionally on every MMIO dispatch, and the per-region opt-out that used to exist
was removed upstream. ★ But the KVM exit site itself is **already outside** that lock — it is
re-taken one level down, in the dispatch helper. ⇒ ★★★ **One hook, at the exit site, before dispatch** — our guest-physical ranges are recognised
and routed into the trap path directly, and everything else falls through unchanged.

⊘⊘⊘ **[DECIDED w821 — and it reverses my own simplification.]** I briefly proposed the smaller-
looking alternative: a flag on the memory region's operations, restoring an opt-out upstream once
had. ⊘ **That is three patches, not one, and two of the three are facilities that do not exist:**

1. the lock opt-out itself — ⚠ and it was removed **as dead code**, so this is new design, not a
   revert, and must be argued upstream on its own merits;
2. the **re-entrancy guard** opt-out, without which two vCPUs racing drop a write silently;
3. ⊘⊘ **a read-only device-memory region whose trapped write reaches our code at all.** There is
   no such constructor. A read-only device region *does* produce the memslot we want, but its
   write handler **stores the guest's value straight through to the host pointer** — so the guest's
   **untranslated** token would reach the real hardware doorbell, ringing the wrong channel or
   none, and our translation would never happen.

★ **The exit-site hook sidesteps all three**, because the access never enters the dispatch
machinery: no lock is taken, no guard is consulted, and no region write handler runs. ⇒ **One
patch, and it is the one that makes §5.7 implementable at all.**
⚠ **Its price, stated:** it must track our own BAR placement and whether memory decoding is
enabled, which the dispatch machinery would have given us for free. That is cheap — **we are the
device model, so we already know both**, and a relocation passes through our own configuration
write path.

⊘⊘⊘ **And the patch must disable one more thing, or it creates the very hole it exists to avoid.**
The VMM guards every device's I/O dispatch with a **plain, non-atomic boolean** — *"do not allow
more than one simultaneous access to a device's I/O regions"*. Under the global lock that only
trips on true re-entrancy. **Without it, two vCPUs writing our device concurrently race on that
boolean, and the loser's write returns an access error and is never performed** — and the exit path
**discards the return value**, so the write is dropped **silently**.

★★★ **That is a write-drop primitive unprivileged guest userspace can aim at guest root.** A
doorbell storm from one vCPU keeps the flag set; guest root's privileged writes on another vCPU
disappear at random — the invalidate trigger (the guest then proceeds with stale translations), the
interrupt acknowledgement (a storm, or a permanently masked engine), the message-queue kick (the
request is never noticed, and enough of those mark the device for reset and **cost the guest its
GPU until reboot**).

⇒ **The same regions must also set the per-region guard opt-out**, and **a region that runs
lock-free with the guard still enabled must fail at startup.** ⚠ The two changes are one change;
shipping the first without the second is worse than shipping neither.
⊘ Without it, every trapped write runs under the same lock as memory-region commits and the main
loop, and the tail argument above is false rather than merely optimistic. ⊘ And the obvious
alternative does not work: the kernel's doorbell-style fast path **discards the data word**, and a
doorbell carries a token we must read.

⚠ **We do not manage vCPU blocking — we stay out of it.** A vCPU blocks routinely and correctly
when the *guest* halts and KVM parks it. What is forbidden is a block **we** impose: the guest did
not ask, cannot see it, and its scheduler believes that core is running.

---

## 4. Two privilege boundaries

The outer one is the usual: the guest is untrusted. The inner one is the design's organising idea.

> ★★★ **Unprivileged guest userspace writes the doorbell directly.** The threat is not the guest
> corrupting itself from root — it is a sandboxed process in the guest reaching guest root.

Three facts make it unfixable by any check at the trap:

1. The doorbell page is **one memdesc per GPU**, DUP'd to every client, 64 KiB, doorbell at `+0x90`
   — and RM waives its own privilege check on it by name, `MEMDESC_FLAGS_SKIP_REGMEM_PRIV_CHECK`.
2. The token is a bare concatenation of runlist id and channel id. No capability.
3. The trap has **no identity to check**. The usermode window is a BAR0-relative register offset,
   and the driver's mmap gate admits it *because* it is inside the register BAR. Guest kernel and
   guest userspace reach one GPA, one memslot. CPL is readable; *which process* is not, and that
   is what an authorization decision would need.

★ **What a hostile ring actually buys is cycles.** The attacker supplies 32 bits and nothing else;
everything we then act on comes from memory only the victim can write. Our cost profile is
hardware's: ring anything, pay a redundant look.

⇒ **So the rule is about reach, not inspection:**

| plane | writable by | therefore |
|---|---|---|
| **doorbell** | unprivileged guest userspace | fixed-size, idempotent, **non-degradable**, needs no identity |
| **privileged registers** | guest root only | an ordered ring that must not overflow — root harms only itself |

⊘ **The test for any trapped page: can unprivileged guest userspace write it?** If yes, it gets no
queue, no degradation mode, no structure shared with another guest process. That set must be
**derived** from which objects RM maps to non-kernel clients, not hand-written — a page missing
from the list defaults to the privileged arm, which is the bug.

---

## 5. The vCPU trap path

Only **writes** trap. Reads are served from ordinary DRAM the guest reads directly, with no exit
and no code — because a vCPU inside an MMIO exit is not preemptible, and driver init polls some
registers thousands of times.

⚠ **Not every read, though.** Roughly **524 of 4096** BAR0 pages must still read-trap — the
framebuffer window resolves through a latch, and the firmware-boot pages are state-machine state.
**The read-trap set is an allowlist, written down, exactly like the write list.**

⊘⊘⊘ **And it is a parity hazard, not just a correctness one.** `[MEASURED, the C artifact]` **99 %
of its 1 000–3 000 exits per token were READS of a single firmware debug register** on a page this
design keeps read-trapped as *"boot state"*. That one page cost a **2.5× loss on LLM decode**. ⇒
**A page is read-trapped for a phase, not forever.** The runtime-polled words on the firmware pages
are shadowed in DRAM; the trap applies during boot, and is dropped when boot completes.

⊘ **The timer page is not one of them.** On Turing and newer the kernel's clock is the pair in the
*usermode* window — which is already the read-only memslot over live host time — and the legacy
page at the old offset is touched only at boot and resume, never read at runtime. ⇒ It is a
**computed shadow**: constants for the privilege mask and tick frequency, and the two time-register
writes **refused by name**, so guest and host cannot drift onto different timebases. ★ One residual
is handled the same way as the doorbell page: if a guest client ever asks for its own CPU mapping of
the timer, install a read-only memslot over ours, lazily, by handle.

```
nvkvm_trap_write(bar, off, val):

  if (bar, off) == chip.doorbell:          # generated per die/arch; Hopper+ maps it over BAR1
        tok = val & chip.token_mask
        w   = &token[tok]
        r   = route_of(w)
        if r == PASSTHROUGH:  write the host doorbell INLINE. No queue, no wake, no lock. Return.
        if r == UNKNOWN:      Return.      # no bit, no bump, no wake
        prev = fetch_or(w, RUNG)                          # AcqRel
        if prev had RUNG:     Return.      # already pending
        set the rung bit; bump work_seq; maybe write(eventfd)
        Return.

  elif chip.userspace_mappable(bar, off):  Return.        # do NOTHING

  else:                                    # privileged: guest root only
        if readable(off): shadow_write(off, val, write_semantics(off))
        append to the privileged ring
        bump work_seq; maybe write(eventfd)
        Return.
```

★ **The middle arm is the one the threat model needs.** The doorbell page is 64 KiB and the
doorbell is four bytes of it; every other offset on it is guest-userspace-writable, and without
this arm an unprivileged process pushes unbounded garbage onto the plane reserved for guest root.

★ **`UNKNOWN` must return doing nothing at all.** Bumping the sequence for an unowned token lets an
unprivileged process keep every worker spinning: workers register as polling only if the sequence
is unchanged, so a token nobody owns, rung in a loop, prevents them ever parking.

### 5.1 Two structures, because one cannot do both jobs

| structure | shape | job |
|---|---|---|
| **token word** | one `u64` per token: route, state, `applied_seq` stamp, host token | every CAS. No neighbour can fail it |
| **rung bitmap** | 1 bit per token + a summary bit per 64 | **scanning only**, never CAS'd |

Sizing comes from what the register can **express**, not what is legal: the decoded fields are
12 bits of channel and 7 of runlist on Ampere, plus a doorbell-type bit on Blackwell — 19 to 21
bits. ⇒ Token words are 4–16 MiB and are **never scanned**; the bitmap is **64 KiB** with a **1 KiB**
summary and is what a worker walks. The token is **masked**, not validated — masking is what the
hardware does with undecoded bits, and it removes an error path.

⚠ **The bitmap is a hint; the word is the truth.** A bit set over an IDLE word costs one look.
⚠ **Clear the summary bit first, then re-read the word.** Clearing after a scan loses a token
that arrived during it.

### 5.2 The token's four states

```
vCPU:    stamp the ring position, THEN fetch_or(RUNG)        # order matters — see §5.6
worker:  CAS RUNG → BUSY        (fail ⇒ not ours)
         act
         CAS BUSY → IDLE        (fail ⇒ BUSY_RUNG ⇒ CAS BUSY_RUNG → BUSY, act again,
                                 bounded: after K rounds, CAS BUSY_RUNG → RUNG,
                                 re-publish, and move on)
         cannot act yet ⇒ CAS BUSY → RUNG, then re-publish
```

⊘ **"Re-publish" means the bitmap bit AND the summary bit, in that order.** Setting only the
summary loses the token **permanently**: the next worker clears the summary, finds the group word
zero, and moves on — and a further ring from the guest returns early because `RUNG` is already set,
so it produces no bit and no wake. ⇒ **Publication is always bit-then-summary, and the trap and the
put-back use the same order.**

⊘ **The re-act loop is bounded, because it is otherwise a livelock an unprivileged guest process
can hold.** A process ringing its own channel in a tight loop keeps a worker in
*act → CAS fails → act* forever; with enough channels it pins every worker and the guest kernel's
own scrub and UVM channels starve — the inner boundary §4 exists to defend. ⇒ After K rounds the
worker republishes and moves on. ★ That is a timeslice, and it is what hardware does for the same
reason.

⊘ **A token needs a retired state, or channel recycling is a use-after-free.** Without one: the
guest frees a channel while its token is `BUSY`, the free path drops the twin, the guest's driver
**recycles the channel id**, and the new channel's first ring lands on a word still carrying the
old route — while the old worker is still reading what it believes is a pushbuffer. ⇒ Free
transitions `IDLE|RUNG → DEAD` and **waits out `BUSY`** before dropping the twin; allocation
transitions `DEAD → IDLE` with the new route.

★ **A bit that says work exists is not a bit that says you may touch the hardware.** Without
exclusion, two workers walk one channel against one cursor: `[c0,p1)` executes twice, or the cursor
moves backwards and the front-end reads it as a ring wrap. ⊘ This is owed to the **victim's own
concurrency** — two legitimate rings race identically. An adversary only widens the window.

⚠ *"Found but unactionable"* counts as **no work** for the purpose of sleeping; otherwise a worker
spins on a token whose enabling register write also needs a worker.

### 5.3 The wakeup word

```
worker_vcpu_poll : u64 = { work_seq : 56 (HIGH) , workers_polling : 8 (LOW) }
```

vCPU: publish, then `fetch_add(1 << 8)`; if the pre-value had pollers, write the eventfd.
Worker: read `seen` **before** scanning; scan; register as polling **only if the sequence is
unchanged**; otherwise rescan. That is `prepare_to_wait`-then-`schedule`, and it removes the
question of who clears a flag and when.

`work_seq` is the **high** half: a carry then falls off the top of the word instead of landing in
the poller count as a phantom poller that makes every later trap pay a syscall. 56 bits is not
about global wrap — it is about wrapping *inside one scan*, and every vCPU CASes the same cacheline,
so the aggregate rate is capped by ping-pong at ~10⁷/s. That is ~228 years.

Orderings are load-bearing and x86 TSO hides their absence: `AcqRel` on the poll word both sides,
`Acquire` on `seen`, `Release`/`Acquire` on indices and claims. The rule: **every write to a work
source happens-before the bump, and every `seen` load happens-before the scan.**

⚠ The eventfd is a **sum, not a queue of wakeups** — `EFD_SEMAPHORE | EFD_NONBLOCK`, and a wake
must reach **one** waiter, not all of them. ⊘ A plain write wakes every non-exclusive waiter, so at
255 workers a single doorbell wakes 255 threads and 254 of them find nothing.

⊘ **And the drainer parks on its own word, not the workers'.** Sharing one word with a
wake-exactly-one policy means a privileged register write can wake **a worker instead of the
drainer** — and the register write then waits for an unrelated doorbell while the guest spins on a
trigger. ⇒ **Two words, signalled by the two arms of the classifier**, which are disjoint by
construction. ⚠ This does not reopen the multi-GPU objection in §9.1: that is about workers
registering on more than one word, and they still register on exactly one.

⚠ **No handler may wait on another queued item**, or a collapsed wakeup becomes a hang rather than
latency.

### 5.4 The privileged ring

One bounded **lock-free MPSC ring**, fixed capacity, preallocated and **prefaulted** — growth means
the allocator on a vCPU. The claim is **conditional**: read the cursor, test fullness, take the
slot by CAS; on full, poison and return **having claimed nothing**. ⊘ A `fetch_add` claim that then
waits for the consumer is the vCPU waiting on a worker in lock-free costume; a claim-then-bail
strands the consumer at that slot forever.

One **dedicated drainer**, spin-then-park, peek → apply → commit, so a failed apply retries at the
head and `applied_seq` is a clean monotonic number other paths fence against.

**Why the order matters at all:** a guest thread on one vCPU writes `PDB_LO`/`PDB_HI` under an RM
lock and releases it; another thread on another vCPU takes the lock and writes `TRIGGER`. On
hardware the first trap returned before the lock released, and PCIe writes to one function are
ordered. Per-vCPU rings preserve only per-vCPU order and fire the trigger against a stale base.

⊘ **Full ⇒ poison the device**, never wait: a sticky error state, further writes dropped, a
guest-visible fault. Dropping one write silently leaves state a later, differently-privileged guest
process inherits.

**Capacity: 4096 entries per GPU**, 64 KiB, prefaulted. The bound is derived, not guessed: every
runtime write funnels through one RM entry point, and RM **serialises its own register sequences
with its own locks**, so in-flight count does not scale with vCPUs. A lock cycle is ~7 writes
(~13 on Turing), an interrupt clear is one, a TLB invalidate is 2–4 **and the guest pre-polls
before writing**, and the RPC kick allows **at most one outstanding**. Realistic maximum: **under
64**. ⇒ 4096 is a 60× margin; sustained occupancy above 25 % is a defect to investigate, and the
high-water mark prints at teardown.

⊘⊘ **One class must never enter the ring, and it would overflow any ring that exists.** On Turing
and GA100 the firmware images are loaded through **auto-incrementing falcon data ports**
(`IMEMD`/`DMEMD`, and Hopper's `EMEMD`), unpolled, ~256 writes per kilobyte ⇒ **16 000–65 000
back-to-back writes** during boot. ⇒ They are classified `data_port` and applied **synchronously on
the vCPU** as a store into the falcon image shadow at the address the companion latch names. That
is a memory store, it is in the privileged arm, and without it the design cannot boot on those
parts.

### 5.5 The shadow

A register the guest can read back is written to the shadow **synchronously, in the trap**, before
anything is queued — because reads come from that page and a deferred write leaves the guest
reading stale state. The sharp case is interrupt masking: the ISR writes the mask then reads
pending, and a deferred mask delivers an interrupt the guest already masked.

⚠ **A plain store is wrong for four of the five kinds**, so the register descriptor carries
`write_semantics`:

| kind | behaviour |
|---|---|
| `plain` | store |
| `w1c` | `fetch_and(!bits)` — a load-store loses a worker's concurrent set |
| `write_only_port` | the readable effect is on a **different** register |
| `trigger` | the guest writes 1 and **spin-reads until 0** ⇒ ★ cleared **after the work is done**, by whoever did the work — see below |
| `data_port` | an auto-incrementing image port ⇒ a synchronous store into a shadow image (§5.4) |

⊘ **`write_semantics` cannot be generated from the published access codes, and that is measured.**
The interrupt-pending register (write-1-to-clear) and its enable-set port carry the **same** code;
the invalidate trigger reads as plain read-write. ⇒ Read/write-only-ness is generated; the rest is
a **hand-maintained per-family overlay of about a dozen names** — the interrupt leaf and its
set/clear ports, the invalidate triggers, the falcon interrupt-clear and transfer-command
registers, and the data ports. ⚠ The overlay is small, but it is hand-maintained, and it must fail
the build when a register in the generated set has no entry.

★ That last row is why the drainer *does* write some guest-writable shadows: it owns their
completion. It must not write one the guest also drives.

⊘⊘⊘ **But the drainer must never WAIT for that completion, and a naive reading of the rule makes it
wait for the slowest thing in the system.** An invalidate trigger completes only when the VA
manager has applied the diff — which waits on the GPU walker and on host mapping calls taken under
the host driver's own lock. ⇒ If the drainer blocks there, **everything queued behind it stalls**:
the message-queue kick, the interrupt re-arm, every later register write. A co-tenant holding the
host GPU lock for a few seconds then lands squarely in the next hazard.

⇒ **The drainer hands the trigger to the thread that does the work and moves on.** It keeps two
counters — what it has *issued* and what has *completed* — and the owning thread clears the shadow
when its work is done. Doorbell workers fence on *completed*, and only for the polled class.

⊘⊘⊘ **And the clear must name what it completes, or a slow invalidate silently corrupts.** The
guest's invalidate spin has a timeout of a few seconds, after which **the driver proceeds anyway,
with stale translations and no error** — that is its documented behaviour, not a bug we can fix.
So: the guest times out on invalidate *A* and continues; it later issues invalidate *B*; our work
for *A* finishes and clears the trigger; the guest reads zero and concludes *B* is done. **It is
not.** ⇒ The shadow carries the ring position of the write it belongs to, and the clear is a
compare-and-set against that position — never a blind store.
⚠ **And add the tripwire:** a trigger outstanding past a fraction of the guest's own timeout budget
**faults the device loudly**. Letting the guest soldier on with stale translations is the failure
mode that produces wrong numbers rather than a crash.

⊘⊘⊘ **A late unmap is a SECURITY event, not a latency problem, and an attacker can cause one.**
The guest's invalidate wait is bounded; ours is not. Landing an unmap means a host call taken under
the host driver's own device lock — and **a hostile channel can hold that lock**, by faulting the
host GPU in a loop and making the driver run recovery. ⇒ The guest times out, scrubs the page, hands
it to another process — **while the attacker's host mapping still resolves to it.**
⇒ ★ **If any unmap in a refresh lands later than the guest's own timeout for that driver version,
tear down every host channel that could still hold the stale translation before applying anything
else**, and count it. A non-zero count is red in the suite, not a warning.

⊘⊘ **And if the HOST's own invalidate times out, we must poison — not clear the guest's trigger.**
The host driver writes the entries and then reports the timeout; the twin's translations are stale
and **there is no undo**. ⇒ Clearing the trigger at that point tells the guest a barrier completed
that did not. **Poison the device instead**: honest, root-visible, and recoverable by reset.

⊘ **A related consequence for the diff: it must be exact, not best-effort.** Mapping over an
already-mapped range returns an out-of-memory status — and the host driver's second overlap check
fires **after** it has already written entries. ⇒ Overlaps are **prevented by the diff**, never
detected by the return code. ⚠ And issue unmaps as
soon as the walker reports a cleared leaf, rather than only at the invalidate.

### 5.6 Doorbells versus registers, across the boundary

Order holds *within* the ring. Doorbells go around it — passthrough hits hardware inline, managed
sets a bit another thread consumes. ⇒ A doorbell can be serviced against pre-write device state.

**The fence:** the doorbell trap stamps the current ring position into the token word, and the
worker services it only once the applied position has caught up. The **worker** waits; the vCPU
never does. ⇒ The doorbell invariant holds: *queued now ⇒ eventually scheduled, never blocks.*

### ⊘⊘⊘ Two different fences, and conflating them made this over-specified

**[CORRECTED w821.]** `[owner]` *"do translated channels also need a fence, and since we just
translate, can the host provide it after translation?"* ⇒ The question separates two things this
section had merged.

| fence | protects against | who can provide it |
|---|---|---|
| **register-order** — a doorbell serviced before a queued register write lands | a *register* affecting how a channel is serviced | us, with the ring position |
| ★ **mirror-currency** — a channel translated against stale mappings | our **VA mirror** being behind the guest's last published invalidate | ⊘ **only us, and only before submission** |

★★★ **The second is the one translated channels actually need, and the first is nearly vacuous for
them.** Translation consults the **mirror**, not the register file. And the enumeration in §5.6
already establishes that on a firmware-client guest **there is no register path for runlist submit
or channel enable** — that rides the message plane — so a queued register write cannot change how
we service a managed channel. The only register that matters is the **invalidate trigger**, and
what that trigger *means* is *"the mirror must catch up."*

### ★★★ How it is specified: as an OBSERVABILITY INVARIANT, not as a check

`[owner]` *"oh, the fences are — it must have invalidated by then. Is that what it is? And how is
it specified then?"* ★ **Exactly that, and stating it as an invariant rather than a check makes
most of the machinery disappear:**

> ⊘ **The guest must never be able to observe an invalidate as complete before the mirror reflects
> it.**

That is one sentence, it is checkable in **one place per transport**, and it cannot be forgotten at
a call site the way a fence check can.

### ⊘ Which fences are forwardable to the host — the rule, and it decides both transports

`[owner]` *"but are fences then not forwardable to the host?"* ★★★ **The rule is one line:**

> ★ **A fence is forwardable when the thing that must wait is the GPU. It is not forwardable when
> the thing that must wait is the guest's CPU.**

| transport | who must wait | forwardable? | mechanism |
|---|---|---|---|
| **register trigger** | ★ the **guest's CPU**, which is spinning on our shadow | ⊘ **no, and it needs no forwarding** — the guest is *already* stalled. There is no channel to hold and nothing for the host to do | clear the trigger only after the apply lands (§5.5) |
| **pushbuffer invalidate** | ★ the **GPU**, executing a stream we submit | ✔ **yes — and it is better than holding it ourselves** | a **semaphore acquire** in the translated stream |

### ★★★ Forwarding the pushbuffer fence: the hardware waits, not us

The channel front-end can **stall in hardware until a memory location reaches a value** — the
same primitive the guest's own driver uses for cross-channel ordering. ⇒ Instead of holding the
remainder of the stream in software:

1. submit the prefix, up to the invalidate;
2. submit a **semaphore acquire** on a location we own, for the mirror generation this stream
   needs;
3. submit the remainder **immediately**;
4. when the apply lands, **we write the location** — and the GPU releases itself.

★ **Use the *greater-or-equal* form, not exact match**, so a later apply also satisfies an earlier
wait and a generation that has already moved past cannot wedge the channel.

⇒ **What this buys over holding it ourselves:** no worker carries state across the wait, no
re-submission logic exists, and the channel resumes **at hardware speed** the instant we signal
rather than at the next scheduling opportunity. ⊘ And it deletes the *"hold the channel's
subsequent entries"* machinery entirely.

★ **There is a pleasing symmetry with completions.** A translated completion is *translated in
address only — the real engine writes the value*. The fence is its mirror image: **we write a value
the real engine waits on.** Same plane, same primitive, opposite direction.

⊘⊘ **And the obligation it creates, which must be honoured on every exit path:** a stalled channel
whose value is never written **hangs** — and on a kernel channel a hang is globally fatal for the
guest's whole GPU stack. ⇒ **Every path that abandons an apply — including poisoning the device —
must still write the semaphore.** Poison must release, not merely stop.

⇒ ★★★ **And this means the register path needs no separate fence at all.** If the guest cannot see
the trigger clear until we have applied, then any doorbell it rings *afterwards* is already ordered
behind our apply — by its own spin, not by our bookkeeping. **The per-token ring-position stamp was
solving a problem the trigger discipline already solves.**

⚠ **The invariant has exactly one failure mode, and it is the security case:** the guest's spin is
**time-bounded** and it proceeds anyway on timeout. That is the one situation where the invariant
cannot be held — and the response is the tear-down-and-count rule above, not a weaker fence.

### ⊘ And the host cannot provide it, because translation happens BEFORE submission

`[owner]` *"can we just rely on the host userspace for providing that one after translation?"*
⊘ **No — and the reason is a one-way door.** By the time the host sees our work, the operands are
**already concrete addresses in our space**. A translation made against a stale mirror does not
arrive at the host as a question it can answer; it arrives as a wrong address. The host then
either faults, or — worse — **reads a mapping that still resolves to a page the guest has since
reused.** That is the cross-process leak, arriving by a different door.

★ **But the instinct is half right, and the half that holds is worth keeping:** the host *does*
provide the **execution-side** ordering. Once we have translated correctly, work on our channel is
ordered against the host's own mapping calls by the host driver, and against other work on that
channel by the hardware. ⇒ **We fence translation; we do not fence execution.** That is why the
fence is one load rather than a round trip.

Three details are load-bearing:

- ⊘ **Stamp before setting `RUNG`, or fold both into one atomic.** They are two read-modify-writes;
  if `RUNG` lands first, a worker can claim the token, read the *old* stamp, believe the fence is
  satisfied, and act against pre-write device state — the exact thing the fence exists to prevent.
- ⊘ **Compare in modular order, not with a maximum.** The stamp shares a word with route, state and
  the host token, so it is narrower than the ring counter. A plain `fetch_max` never wraps back,
  so after the stamp space rolls over the token becomes **permanently unserviceable** — a hang that
  appears after weeks of uptime and nothing else explains. ⇒ Compare as a signed difference and
  store-if-ahead in modular order.
- ⊘⊘ **The drainer must wake the workers after it commits.** Otherwise: a worker claims a token,
  finds the fence unsatisfied, puts it back and parks; the drainer applies the write and advances;
  **nothing rescans**, because parking is only ever ended by a new bump. ⇒ A guest that rings once
  and waits on a semaphore — any single stream synchronise — hangs. The drainer bumps the sequence
  and signals after every batch that advances past a stamped position.

★★★ **Passthrough needs no fence, and the obligation is discharged by enumeration.** Every
privileged runtime write on a GSP client falls into one of four classes, and none of the first
three can change how hardware services a channel at the moment of a ring:

| class | why it cannot matter |
|---|---|
| **polled** | the guest spins on a read-back — invalidate triggers, L2 invalidates, falcon transfer and control. ★ The drainer clears the polled bit **only after the host-side effect is complete**, so the guest cannot ring until it observes the clear. **Self-fencing.** |
| **rpc-fenced** | the message kick allows **one outstanding** and the guest waits for a reply we send after acting. ⇒ Channel alloc, bind, schedule and preempt all ride here — ⊘ there is **no register path** for runlist submit or channel enable on a GSP client |
| **irrelevant** | interrupt tree, timers, fault-buffer cursors, mailboxes, data ports |
| **refused** | the fault/recovery registers — channel-RAM and runlist preempt. ⊘ **Unimplementable from host userspace anyway**, and already excluded by *delivery, never recovery* |

⇒ **[PROPOSE] Make it a build-time property, not a proof in prose:** every privileged register in
the generated descriptor set carries a `fence_class`, and **an unclassified register fails the
build**. A new die then cannot reopen the hole silently. ★ Keep the managed-channel stamp as a free
tripwire and assert the wait count is **zero** at teardown — a nonzero count means the enumeration
is wrong.

### 5.7 The doorbell page itself

Mapped as a **read-only KVM memslot over the real host doorbell page**: reads are hardware with no
exit, writes trap into the path above, and when *we* ring we write the host page directly.

On Hopper and newer, unprivileged clients map the doorbell over **BAR1** instead — `bBar1Mapping`
is not privilege-gated, and both UVM and NVIDIA's own pushbuffer library request it. ⇒
**BAR1 is never trapped, with exactly one exception: the page the guest maps the usermode object
at.** We identify it by **RM object handle** — we serve the allocation, so we hold the handle and
the flag — and use the page-table diff only to confirm. Identifying it by "it looks like the VF
register block" is a pattern match on guest-chosen data.

★ On Hopper+ the guest kernel still rings BAR0 while userspace rings BAR1 — **two pages, two GPAs,
two privilege domains.** Where the hardware gives us that separation, use it.

---

## 6. Memory and addresses

**Guest video memory is one host allocation**, sized by a command-line parameter like guest RAM.
The guest framebuffer offset **is** the offset into it. Guest system memory is the hypervisor's
memfd through a static layout.

**Mirror, never adopt.** We reproduce the guest's page tables in our own; we do not take over its.
Three independent reasons: adopting races the host driver's own tables and invalidates; the host
driver exposes no userspace interface to participate in its locking, so adopting needs a host
kernel module and a deployment bottleneck; and a translation is needed anyway, because guest
physical is not host physical. ⇒ Mirroring is also **rootless**, which is the posture the product
needs.

**The diff.** A GPU-side walk of the guest's page tables produces a per-VA-space delta — which
spaces changed and which entries to apply. Previous state lives in video memory, held by the walker
itself. The VA manager executes the delta as `mmap`/`munmap`, batched with the TLB-defer flag on
every call but the last. ⚠ The defer batches the barrier; it does not remove it — a refresh still
happens last, **and it is issued unconditionally, including on a failed batch.**

⊘⊘⊘ **There is more than one publish trigger, and the one the design named is not the one CUDA
uses.** The register-based invalidate is how the guest *kernel* publishes. But the unified-memory
path writes its page tables **with the copy engine** and issues its invalidate **as a pushbuffer
method** — it never touches that register. ⇒ A design triggered only by the register **never
mirrors a managed-memory unmap**, and the freed guest page is handed to another process while the
first process's host mapping still translates to it. **That is the cross-process leak, reached by
ordinary CUDA, with no race required.**

⇒ ★ **The decoded invalidate method on a translated kernel channel is a first-class publish
trigger**, carrying the same barrier as the register path: the pushbuffer position is held until
the unmaps have landed.

⊘⊘ **And the method must be stripped from the forwarded stream, not passed through.** It is a
**privileged** method: a channel that is not privileged **faults** when it executes one, and our
host twins are unprivileged by construction — the host driver grants that privilege only to kernel
or administrative clients. ⇒ Forwarding it would fault our own twin. We consume it as a trigger and
do not submit it.

⚠ **The walker competes with the workload it mirrors.** Different contexts on one GPU time-slice
at roughly a millisecond, so while a guest kernel is running, a 200 µs walk can wait a full slice
and costs two context switches. ⇒ Either run the walk on a **copy engine**, which does not preempt
compute, or accept the floor and state it. ⊘ It must not be discovered as *"invalidates are slow
under load"*.

⊘⊘ **The walk must be ordered against the guest's own page-table writes, and one case is not
naturally ordered.** The guest's memory manager writes video-memory page-table entries **with the
copy engine, inside a pushbuffer**, and issues its invalidate **in the same pushbuffer on the same
channel**. ⇒ If we trigger the walk when we *decode* the invalidate method, we read the tables
**before the engine has written them.** The split is: submit the pushbuffer prefix up to the
invalidate, **wait for its completion on our twin**, walk, then continue. ★ This is an ordering the
design assumes and must establish.

### 6.1 ⊘⊘⊘ The cost of applying a diff is a HOST-GLOBAL lock, and it decides the allocation path

**Every mapping call takes the host driver's single driver-wide API lock in WRITE mode**, plus a
per-GPU group lock. ⇒ Every other client on that host — **other VMs, and host CUDA** — serialises
behind each call we make. The deferred-invalidate flag elides one flush and one whole-space
invalidate; it does **not** touch the locks, the address-space allocation, or the five kernel
allocations per mapping.

`[MEASURED]` on the current tree: **13 313 mapping plans for a 1 GB model**, ~132 µs per call. ⇒
Applied naively, **a model load is tens of seconds of wall clock**, and it is wall clock stolen
from every other tenant.

⊘⊘⊘ **And on Windows this is not a performance question — it is whether the product works.** That
guest maps system memory in **4 KiB** units and gives the driver a **hard two-second deadline**
before it resets the adapter. At ~130 µs per mapping call, a one-gigabyte mapping is a quarter of a
million calls — **tens of seconds**. ⇒ **Without coalescing, a Windows guest cannot work at all**,
and with it the cost is bounded by the number of *runs* rather than of pages.

⇒ ★★★ **Three changes, and they are the difference between a usable product and a demo:**

1. **Coalesce runs.** Slices of one object are contiguous across leaves; map the run, not the leaf.
2. **Reserve the address range at allocation**, using the object class that does so, rather than
   letting every map allocate. ⚠ It also removes a trap: re-mapping an already-mapped address
   currently returns an out-of-memory status, and reading that as *"already mapped, success"* is a
   coincidence, not a contract.
3. ★★★ **Register the guest-RAM memfd ONCE, at startup, and map sub-slices of it.** The per-page
   pinning cost then happens **one time** instead of per mapping — the same posture device
   assignment already takes. This is the single largest item on the list.

⚠ **And guest RAM must be backed by huge pages.** The host driver **silently downgrades** a
large-page mapping whose backing is not physically contiguous, so a guest on ordinary 4 KiB pages
gets 4 KiB entries on the host GPU — sixteen times the mapping count and sixteen times less
translation reach. ⇒ A startup requirement, checked, not a recommendation.

⊘⊘ **And the diff is two-phase, or a partial failure desyncs the mirror permanently.** If the
walker commits its new state when it walks, and the VA manager then fails part-way through — host
address space exhausted, an allocation refused — the unapplied entries are **never retried**, and
the mirror believes they are done. ⇒ The walker commits **only what the VA manager acknowledges**.

**A VA space is the object; the page-directory base is an attribute of it, per GPU.** RM's identity
is the object and a monotonic unique id; the base is per-GPU, mutable (relocation is explicitly
repeatable), can be **absent** — hardware encodes that case — and is not unique. ⇒ Identity is the
object; carry the base as a mutable, possibly-absent, per-GPU attribute used to seed a walk.

**We write the roots**, because the page-directory update callback is a no-op inside a guest on
GSP parts — instance blocks are written by the firmware, which is us. Where each root comes from:

| space | root arrives as | aperture | walked by |
|---|---|---|---|
| user VA space | a control carrying the **reserved-PDE table**, whose level-0 entry **is** the top-level directory. Sent for every space, because the split-VA-space default is on | declared in the same message | the GPU walker (vidmem) / CPU (sysmem) |
| UVM's external root | a control carrying `{physical address, entry count, aperture, VA space, channel}`. ⚠ **It arrives as a control, not as the dedicated message** — that message is stubbed out on GSP clients | in the message | same |
| **BAR1** | ⊘ **ours.** We allocate it and declare the address; the guest adopts it and writes entries directly. **No message exists** | ★ framebuffer *to the guest*; **host RAM underneath** — see below | CPU, at RAM speed |
| **BAR2** | ⊘ **ours.** We declare the address; the guest sends back its **encoded entry value** for slot 0 | same | CPU, at RAM speed |
| instance block | ⊘ **not a root.** The guest allocates it and tells us where; **we** write the base and aperture into it | same | never walked |

### 6.1 ★★★ BAR1 and BAR2 are ordinary VA spaces. There are no split apertures.

`[owner, w821]` *"BAR1/BAR2 do not create multiple apertures per vidmem. If the guest says map
vidmem here, we just follow it — a dumb map — whether it is for rings or for tables. The PTX walker
walks the BAR page directories and tables too, since it has direct access and native performance.
It returns the diff, also for the BAR spaces, and we apply them like any other. It just keeps track
like a normal VA space."*

★★★ **Take this. It is strictly simpler than the alternative and it dissolves a contradiction.**

⊘ **What it replaces.** An earlier draft argued the BAR tables are *metadata nobody but us reads*,
so they should live in host memory and be walked by the CPU. ⚠ That reasoning was sound about who
reads them and **wrong about what it costs**: the guest writes those entries *through BAR2*, so
routing them to host memory means **intercepting BAR2** — which is precisely the interception this
simplification removes. ⇒ The placement trick bought a CPU walk and paid for it with a served
aperture. **Bad trade.**

**The model, stated once:**

- ⊘ **No split apertures.** The guest says *"map this here"*; we follow it. A **dumb map**, whether
  the target is a ring, a page table, or a buffer. Nothing inspects the purpose.
- ★ **The BAR spaces are VA spaces like any other.** Their roots are ours — we allocate them and
  declare the addresses, and the guest adopts them — but the tables themselves live in video memory
  with everything else, and **the walker walks them.** It already has direct access at native
  speed; a few more tables cost nothing.
- ★ **One diff, one apply path, one tracking mechanism.** The BAR spaces appear in the same delta
  as user spaces and are applied as mappings the same way. ⊘ No special case, no second walker, no
  placement decision.
- ⇒ ★★★ **And the contradiction is gone.** BAR2 is neither *trapped* nor *served* — it is
  **mapped**, and we learn what the guest did with it the same way we learn everything else: the
  walker, driven by the publish trigger.

### 6.2 The one exception, and the lock order it implies

⊘ **The doorbell page is the only thing that becomes a memory slot** — read-only over the real host
page, so reads are hardware and writes trap (§5.7). Everything else is a mapping.

⚠ **Changing a slot requires the VMM's global lock** — `[VERIFIED]` both the region-commit path and
the kernel slot-update path assert it. ⇒ Two consequences, and the owner named the first:

1. ★ **It is a slow path, and that is fine** — the slot is installed once, at device setup, or at
   the first allocation of the usermode object on architectures that map the doorbell over BAR1.
   ⚠ **Not literally "init only" there**: that allocation happens at *process* start, so the
   install is at *first* such allocation rather than at VM start. Still slow-path, still under the
   lock, still not on any hot path.
2. ⊘ **It fixes a lock order.** The thread that installs a slot takes the global lock, so **nothing
   that may be held while taking that lock may ever be taken while holding it.** In this design
   only the VA manager installs slots and the vCPU path takes nothing, so there is no inversion —
   but the rule is stated because the next person to add a lock will not re-derive it.

### 6.1 ⊘⊘⊘ The cost of applying a diff is a HOST-GLOBAL lock, and it decides the allocation path

**Every mapping call takes the host driver's single driver-wide API lock in WRITE mode**, plus a
per-GPU group lock. ⇒ Every other client on that host — **other VMs, and host CUDA** — serialises
behind each call we make. The deferred-invalidate flag elides one flush and one whole-space
invalidate; it does **not** touch the locks, the address-space allocation, or the five kernel
allocations per mapping.

`[MEASURED]` on the current tree: **13 313 mapping plans for a 1 GB model**, ~132 µs per call. ⇒
Applied naively, **a model load is tens of seconds of wall clock**, and it is wall clock stolen
from every other tenant.

⊘⊘⊘ **And on Windows this is not a performance question — it is whether the product works.** That
guest maps system memory in **4 KiB** units and gives the driver a **hard two-second deadline**
before it resets the adapter. At ~130 µs per mapping call, a one-gigabyte mapping is a quarter of a
million calls — **tens of seconds**. ⇒ **Without coalescing, a Windows guest cannot work at all**,
and with it the cost is bounded by the number of *runs* rather than of pages.

⇒ ★★★ **Three changes, and they are the difference between a usable product and a demo:**

1. **Coalesce runs.** Slices of one object are contiguous across leaves; map the run, not the leaf.
2. **Reserve the address range at allocation**, using the object class that does so, rather than
   letting every map allocate. ⚠ It also removes a trap: re-mapping an already-mapped address
   currently returns an out-of-memory status, and reading that as *"already mapped, success"* is a
   coincidence, not a contract.
3. ★★★ **Register the guest-RAM memfd ONCE, at startup, and map sub-slices of it.** The per-page
   pinning cost then happens **one time** instead of per mapping — the same posture device
   assignment already takes. This is the single largest item on the list.

⚠ **And guest RAM must be backed by huge pages.** The host driver **silently downgrades** a
large-page mapping whose backing is not physically contiguous, so a guest on ordinary 4 KiB pages
gets 4 KiB entries on the host GPU — sixteen times the mapping count and sixteen times less
translation reach. ⇒ A startup requirement, checked, not a recommendation.

⊘⊘ **And the diff is two-phase, or a partial failure desyncs the mirror permanently.** If the
walker commits its new state when it walks, and the VA manager then fails part-way through — host
address space exhausted, an allocation refused — the unapplied entries are **never retried**, and
the mirror believes they are done. ⇒ The walker commits **only what the VA manager acknowledges**.

**A VA space is the object; the page-directory base is an attribute of it, per GPU.** RM's identity
is the object and a monotonic unique id; the base is per-GPU, mutable (relocation is explicitly
repeatable), can be **absent** — hardware encodes that case — and is not unique. ⇒ Identity is the
object; carry the base as a mutable, possibly-absent, per-GPU attribute used to seed a walk.

**We write the roots**, because the page-directory update callback is a no-op inside a guest on
GSP parts — instance blocks are written by the firmware, which is us. Where each root comes from:

| space | root arrives as | aperture | walked by |
|---|---|---|---|
| user VA space | a control carrying the **reserved-PDE table**, whose level-0 entry **is** the top-level directory. Sent for every space, because the split-VA-space default is on | declared in the same message | the GPU walker (vidmem) / CPU (sysmem) |
| UVM's external root | a control carrying `{physical address, entry count, aperture, VA space, channel}`. ⚠ **It arrives as a control, not as the dedicated message** — that message is stubbed out on GSP clients | in the message | same |
| **BAR1** | ⊘ **ours.** We allocate it and declare the address; the guest adopts it and writes entries directly. **No message exists** | ★ framebuffer *to the guest*; **host RAM underneath** — see below | CPU, at RAM speed |
| **BAR2** | ⊘ **ours.** We declare the address; the guest sends back its **encoded entry value** for slot 0 | same | CPU, at RAM speed |
| instance block | ⊘ **not a root.** The guest allocates it and tells us where; **we** write the base and aperture into it | same | never walked |

### 6.1 ★★★ The guest's BAR tables are metadata. Nothing but us ever reads them.

⚠ A CPU read of **real** video memory runs at roughly **48 MiB/s** — flat. ⇒ That is why the
**walker**, not the CPU, reads page tables: it has direct access at native speed. ★ And it is why
§6.1 puts the BAR tables through the same walker rather than inventing a placement that would let
the CPU read them cheaply — the cheap CPU read was never worth the interception it required.

★ **This is also why the GPU-side walker is not the answer here**, even though it is the right
answer for user VA spaces. Those tables are **guest-allocated in real video memory** — we do not
choose their placement, and a CPU walk of them is the 48 MiB/s path the walker exists to avoid. ⇒
**Two page-table populations, two mechanisms, and the discriminator is who chose the placement:**

| tables | placed by | live in | walked by |
|---|---|---|---|
| user VA spaces, and everything the guest allocates | ⊘ the **guest** | real video memory | ★ the **GPU-side walker** |
| BAR1, BAR2, instance blocks | ★ **us** | host RAM | the CPU, at RAM speed |

⚠ **The trap this closes:** *"it is at a framebuffer offset"* and *"it is in video memory"* are not
the same statement in this design. The first is what the guest is told; the second is a placement
decision we make. Conflating them is what makes a CPU walk look unavoidable when it is not.

⚠ **Aperture decides the verb, but it is not the whole leaf.** A page-table entry carries kind,
read-only, atomic-disable, privilege, volatility and a peer id — and the host mapping must carry
them too. ⇒ The walker's leaf record is `{aperture, address, kind, ro, atomic, priv, vol, peer}`,
and the VA manager maps flags from it. ⊘ A leaf record that is only an address will produce
mappings that are wrong in ways that fault late.

★ **The verbs**, once the aperture is known: **vidmem** is a fixed-offset DMA map of a slice of the
single store — the backing already exists, so nothing is allocated; **sysmem** is an OS-descriptor
object over *our* host pointer into the guest-RAM memfd, plus the same fixed-offset map. ⊘ The
guest's own pointer never appears in a host call, which is why the memory class that would carry
one stays refused by name. **Peer** is a third verb and belongs with multi-GPU.

⊘⊘⊘ **The system-memory resolution must admit ONLY true guest RAM, and this is a security boundary
rather than a correctness one.** Our own structures — the register read shadow, the doorbell
bitmap, the boot pages — are host memory installed as guest-physical memslots. ⇒ A guest page-table
entry naming one of *those* addresses, resolved by a layout that maps guest-physical to host
pointers generically, would **pin our own state and hand it to the GPU as a DMA target.** The guest
could then have the engine write the bits the drainer owns.
⇒ ★ **The layout resolves guest RAM blocks and nothing else. Any other guest-physical address is a
refused leaf, by name.** ⚠ Same shape as refusing the memory class that carries a caller pointer:
*the guest may name its own memory, never ours.*

⊘ **And that class refusal is decorative — this is the real guard.** The class never reaches us; it
is handled inside the guest. ⇒ **The only path by which a guest-chosen address reaches a host call
is this leaf**, so it is the one that must carry the bound. The design states a bound for
video-memory leaves; **the system-memory leaf needs the same bound stated, not implied.**

⊘ **The leaf's other bits are an allowlist, not a translation.** Forwarding a guest-chosen page
**kind** would mint host mappings from a guest value and touch device-global compression state — a
guest-to-host coupling in the same family as forwarding a flag word. ⇒ We translate **aperture,
address, read-only and page size**; every other bit is **refused by name and counted**, and the
kind is fixed to the store's.

### 6.1 BAR1 and BAR2 are split between us and the guest

★★★ **The guest owns PDE3[0]; we own PDE3[1]; and our table is the one hardware walks.** The guest
rewrites its own page-directory cache to **our** address, which it learns from our static-info
reply. So:

- We allocate **both** root pages in reserved framebuffer and **declare their addresses** in
  `GET_GSP_STATIC_INFO`.
- The guest's TLB invalidates then name **our** base; our BAR2 walker roots at our page with entry
  0 set from the value the guest hands us.
- ⊘ **BAR1 has no update message at all** — the guest adopts our root page and writes entries into
  it directly.
- ★ **The guest writes every user page-table entry through BAR2**, using it as its own window onto
  instance and table memory. ⇒ It is **mapped, not trapped and not served** (§6.1) — trapping it
  would put a page-table fill through the privileged ring and overflow it on a single large
  mapping, and serving it would reintroduce the interception §6.1 removes. **We map it and let the
  walker observe the result.**
- ⊘ **Nobody sparsifies BAR1 unless we do.** An unpopulated entry there is sparse, not an error.

⇒ The framebuffer layout — root pages, the protected firmware region, reserved rows — is **ours to
declare, before static info can be answered.**

---

## 7. Channels

A channel is a pushbuffer, a GPFIFO ring of 8-byte entries, and a small block holding the ring
cursors. The guest appends an entry, bumps its producer cursor, and rings. ★ The consumer cursor
lives in memory mapped to the owning process and nobody else — which is why hardware can treat a
hostile ring as harmless, and why we must too.

| kind | contract | what happens |
|---|---|---|
| **Passthrough** | ring and return; **may** run on the vCPU | the guest's ring is in real GPU memory; hardware fetches it. ⊘ We never read a byte of it |
| **Translated** | schedule and return | every operand rewritten into our VA space, submitted on our host channel. **The GPU moves the bytes**; the completion is translated in address only |
| **Emulated** | schedule and return | we implement the function |

**Birth is at allocation, never lazy** — RM zeroes a caller-supplied cursor block at allocation, so
adopting at first doorbell wipes the cursor that just rang.

**Kernel channels are translated, not emulated.** The scrubber and UVM's own channels do real work
on real memory; running them on our CPU is how a guest process reads another's freed pages.

⊘⊘⊘ **An untranslatable operand must never become a deliberate fault on a kernel channel.** The
unified-memory driver treats **any** channel error as **globally fatal** — one fault kills CUDA for
**every process in the guest** until the driver reloads. ⇒ A design that forwards a translation
miss as a sentinel fault hands unprivileged guest userspace a way to kill the whole guest's GPU
stack. **On a kernel channel we refuse the submission and poison the device** — root-visible and
honest — rather than faulting it. ⚠ And any guest memory region we cannot back must be **refused at
startup**, not discovered as a translation miss at run time.

⚠ **Sizing and decoding are different jobs.** We *size* every pushbuffer method form so the stream
never desynchronises, and *decode* only what we model. An undefined form decodes to nothing rather
than to a guess. And a method is not a unit of meaning — a copy is five method runs and the launch
carries no operands, so only a stateful walk over a run produces a fact.

---

## 8. Completions and interrupts

| kind | completion |
|---|---|
| Passthrough | nothing. We do not inspect the channel and do not care when it finishes |
| Translated | nothing. The semaphore address was forwarded; the GPU writes it |
| Emulated | forged at the end of our own call — there was no GPU work |

⊘ **"Forge" is licensed only where there was no work.** A completion written for work that did not
happen is how a scrub becomes a leak.

⊘⊘ **What we may send the guest is an allowlist, enforced at build time.** We know one message
crashes the host operating system on a Windows guest — but it is not the only one that hurts: at
least three other outbound messages and paths cause the guest's driver to **mark its device for
reset**, which costs the guest its GPU until it reloads. One of those paths is simply **letting a
reply take too long**, repeatedly.
⇒ ★ **Three messages are permitted — boot-complete, the generic completion carrier, and
channel-teardown — and the build fails on any other.** ⚠ And every synchronous reply carries an
internal deadline **well under** the guest's own patience, because the timeout path is reachable by
anything that starves the thread drawing replies.

**Interrupt registration is per engine**, not per channel and not per semaphore: the event object
requires a subdevice notifier, the index names an engine, and the channel class has no completion
event at all. ⇒ An interrupt is a **broadcast wake to every waiter on that engine**, carrying no
identity.

★ **The waiter owns the race**, three independent ways in RM's own code: its semaphore-surface
registration re-reads under a lock and returns *already-signalled*; callers loop on that; and the
wait path never trusts the event, looping on the notifier word and using the event only to decide
when to re-check. ⇒ We fire on **genuinely new completed work** and need not fire retroactively.
⚠ The converse binds: once armed, a later release **must** produce an interrupt. There is no
level-triggered fallback.

★ The channels we care about most **poll** — UVM spins, and the scrubber's blocking waits loop on
the semaphore word.

### 8.1 ⇒ We forward the host's edge, and never decode the request method

⊘ **We do not decode the pushbuffer method by which the guest requests an interrupt.** Three facts
make it unnecessary:

1. **The guest learns its vector from us, not from hardware.** Non-stall vectors are reported only
   through a control we serve; the device-info table carries no vector at all.
2. **Servicing is one register write** — a write-1-to-clear of the pending bit — followed by a
   **per-engine broadcast** to every registered waiter. No per-channel work, no message.
   ★ Enables are set once at state load and never masked at runtime, so the pending bit is a sticky
   latch behind a permanent enable.
3. **The method executes on the host GPU** for passthrough and translated channels, and the **host
   driver broadcasts the same edge** to every registered host waiter.

⇒ **Mechanism:** our host client registers a per-engine non-stall event on the host twin for each
copy engine, graphics and host, and those descriptors join the drainer's poll set. On wake we set
the pending bit for the vector **we declared**, and raise the guest interrupt if it is enabled.
Emulated channels set the same bit at the end of their forged completion.

★ **Why this is correct rather than merely convenient:** the guest's contract is *"the engine
advanced ⇒ broadcast; the waiter re-checks memory."* Forwarding the host's edge yields a
**superset** of the required edges — other host clients' work on that engine also wakes us, which
is redundant re-checking, and is exactly what hardware does across processes. Missing edges are
impossible while the host driver notifies us at all.

⚠ **Two invariants close the known holes:**

1. When the guest re-arms the top-level enable, **re-evaluate pending and re-raise** if an enabled
   bit is still set. The host's non-stall path is strictly one-shot, so nothing else recovers a
   dropped edge.
2. ⊘⊘ **The same applies to a masked interrupt vector, and this one is routine.** The kernel's
   direct-injection path delivers **regardless of the guest's mask**, and Linux masks a vector
   whenever it changes interrupt affinity. ⇒ An edge arriving while masked is **lost**, and §8
   already says there is no level-triggered fallback to recover it. ⇒ **Unmasking a vector must
   re-evaluate pending and re-raise**, exactly like the enable does.

⇒ **This is deferrable past the first compute**, because the things that would need it poll. ⚠ It
is deferrable **only if the absence is red rather than silent**: the suite gets an arm that uses a
blocking-sync wait, so the gap fails a test instead of hanging a workload.

---

## 9. The host side

Host RM objects are owned in-process now that isolates are gone. **We author every host call**; our
host-verb signatures do not accept a guest flag word, so a guest-chosen bit cannot reach RM's
interpretation of it.

⚠ **Host RM recovers channels whether we want it or not.** A fault on our twin gets the channel
torn down by the host driver, and the guest must be told through the mechanism it already
understands, then allowed to run its own recovery and re-adopt.

★★★ **Lifetime: two host descriptors, two lifetimes, and the kernel does the freeing.**

| descriptor | lives for | owns |
|---|---|---|
| **A** | the VM | the single store, the host register mappings, the walker's GPU context |
| **B** | one guest **driver instance** | every twin — devices, VA spaces, channels, engine objects, events, mappings |

⊘⊘⊘ **But `close()` is not a synchronous free, and relying on it leaves exactly the hazard it was
supposed to close.** Two mechanisms defeat it:

- **`release` runs on the last file reference**, not on `close`. Every in-flight call on that
  descriptor and every mapping made through it holds one. ⇒ `close` returns while teardown has not
  started.
- ⊘ **The driver's close path defers the whole cleanup to a kernel thread** when its interruptible
  wait fails — which **a pending signal causes**. VMM threads carry signals routinely. ⇒ The
  deferred path is not an edge case; it is the normal case under load.

⇒ ★ **So we free explicitly and then close.** Unmap every mapping made through the descriptor, join
every in-flight call, **block signals**, issue the **explicit free of the client tree** — which is
synchronous and ordered by the host driver's own dependency rules, preempting and unbinding
channels — and only then close the descriptor.

★ **Why this matters rather than being hygiene:** a still-scheduled host channel whose ring lives in
guest RAM the guest has since reused is **a live DMA engine writing into another guest's memory**.
The guarantee has to be *"the engine is stopped before the memory is reused"*, and only the explicit
free gives that ordering.

**Triggers:** the guest's unload message, a device reset (`system_reset`, function-level reset,
`reboot -f`), and re-entry of driver init with the protected region still up. Our own order is:
quiesce workers, reset the ring, drain the VA manager, drop twin references, **then** close.
⊘ On VMM exit or crash the kernel closes both descriptors and the host driver frees everything —
**a leak across runs is not possible without a host driver bug.** ⚠ That is true of *leaks*; it is
not a substitute for the ordered teardown above, because a crash gives us no chance to run it.

⊘⊘ **And teardown must reset the walker's own state.** Its previous-state snapshot lives across a
driver instance; if it survives while every mapping is dropped, the next instance's first diff
reports *"unchanged"* and maps **nothing** — a second boot that faults for reasons the first did
not. ⇒ Resetting it is part of the teardown order, and the gate is the design's own rule: **run
every gate twice in one process.**

### 9.2 ⚠ Per-VM caps, because a shared host GPU is a shared resource

The host driver enforces no per-client quota. ⇒ On a host with more than one VM, one guest can
starve the others by allocating twins — channels, engine objects, address spaces, and one host
object per non-coalescable system-memory leaf. ★ **We already told the guest how many channels it
may have**, so that number is the cap, and the same applies to every other twin class: **refuse
past it, by name, counted.**

⚠ **And the guest's framebuffer aperture is a device-global resource on the host.** The window
through which a CPU sees GPU memory is a fixed size shared by every process on that GPU. ⇒ Size the
guest's at startup **from what is actually free**, and make an unbackable page a **named refusal**
rather than a silent hole — the guest reading an unbacked hole is a wrong answer, not an error.

### 9.4 ⚠ Two host-platform prerequisites, stated because they are not ours to fix

- ⊘ **Fine-grained runtime power management must be off for a GPU we drive.** The host driver
  **revokes every user mapping** when it powers the device down. Afterwards our inline doorbell
  write faults into a handler that stalls until a scheduled wakeup completes — a block **we** impose
  on a vCPU — and a read-only memslot over the same page makes the guest's exit return an error the
  VMM treats as fatal. ⇒ A host prerequisite, plus a revocation flag the trap checks before touching
  any host page.
- ⚠ **Ballooning and memory hot-unplug must be disabled** for the guest, because we pin guest pages
  for the GPU. Discarding a pinned page is silent guest data loss.

### 9.1 Multi-GPU: what is per device and what is per process

| per GPU | per VMM |
|---|---|
| the register shadow and classifier, keyed by `(gpu, bar, offset)`; token words and rung bitmap (token spaces overlap across GPUs); **the ring and its drainer** — ordering is per PCI function; the boot state machine; the interrupt tree; the host twin and its events; the single store; the walker's GPU context; both BAR root pages | the **worker pool**; ★ **one** wakeup word and one eventfd — a worker registers on exactly one word, and N words would need N atomics and reopen the lost-wakeup proof; **one** VA manager, which also gives cross-GPU ordering for peer mappings for free; the object graph, since client handles are one namespace |

⊘ **Refuse broadcast device groups by name.** The *"one VA space, many bases"* case arises only
there, and CUDA never uses it.
★ **Peer access is a third leaf verb**: an entry whose aperture is *peer* and which carries a peer
id maps as a peer association between our twins plus a fixed-offset map of the other store's slice.
The guest's peer-id space is **ours to define**, because we serve the object that creates it.

### 9.2 Platform decisions

- ⊘ **A guest IOMMU is detected and refused at device realize.** Every guest-supplied system-memory
  address we consume — message rings, page-table entries, cursors, semaphores, and the walker's own
  reads — resolves through a guest-physical-keyed layout, i.e. **bypasses a guest IOMMU by
  construction**. Supporting it means a translation layer plus unmap tracking on every consumer,
  including a GPU kernel that cannot call the VMM. ★ No CUDA or LLM guest needs one, and the
  defaults do not enable one. ⚠ The product case that would reopen this is **Windows DMA
  protection**, and it is an OS-axis question, not a memory-model one.
- ★ **The GPU-side page-table walker runs in-process**, initialised **only from the VA manager
  thread** — the VMM blocks signals around thread creation, and the vCPU signal must not be
  inherited by a library's own threads. It initialises at or after device realize, because the VMM
  daemonises before that. ⚠ Under a sandbox profile that denies process spawning, the device nodes
  must already exist; that is a host prerequisite either way, and it is the one part of this not yet
  measured. ⊘ The fallback if it ever fails is a CPU walk at ~48 MiB/s — **a degraded mode, refused
  by name for any parity claim**, not a supported path.

---

## 10. What is deleted, and why

Isolates and the whole IPC plane · the sandbox plane · per-process containers · the address table ·
joins · VA→phys translation on the submission path · publication epochs and the dirty gate · the
CPU copy executor · the completion watch · the framebuffer probe · every on/off flag for something
that no longer has two sides.

⊘ **These are not edits inside the existing files** — they are the removal of the ideas those files
are built around. Deleting them incrementally means running two architectures in one file set,
which is how contradictory defaults got there in the first place.

★ **Keep measurements and gates; audit prose.** A comment asserting a default will rot. A comment
asserting a dated measurement will not.

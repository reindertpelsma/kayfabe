# GL11 — the per-region "no lock at all" arguments

> **GL11 (normative, `guest_memory_lock.md` §3.4).** A region is registered `LockPath` only
> with a **written argument** that the caller cannot be restructured to revalidate instead.
>
> This file is that written argument, per region. A region with no entry here **must not be
> registered `LockPath`**. Status: **DRAFT — the GSP-queue entry is the load-bearing one and
> it is written against the protocol, not against running code, because `kayfabe-gsp` is
> still a 34-line skeleton.** Every claim is tagged; re-check the tagged-`[inferred]` ones
> when the GSP crate exists.

## 0. The rule this rests on, stated as the owner did

> *"If regions truly have no security lock, you can only do **one** safe memcpy on them before
> operating on the buffer — otherwise it changes underneath."*

That is the whole discipline, and it is worth being exact about **why one copy is enough**,
because the reason bounds what "enough" means:

1. **A race the guest can win by supplying different bytes is not a security problem** if we
   would have accepted those bytes anyway. The guest is always free to send us any legal
   command; racing itself only changes *which* legal command it sent.
2. **The security problem is exclusively re-reading a value after validating it.** If we
   bounds-check a length and then read the length again, the guest can make the check and the
   use disagree. That is the whole TOCTOU class, and it is a property of *our code shape*, not
   of the guest.

⇒ **The invariant that replaces the lock:** *every value used after a check comes from the same
private copy the check was performed on, and no guest address is re-read once a decision has
been derived from it.* Copy once, validate the copy, act on the copy.

**★ Where copy-once genuinely cannot reach**, and therefore where a lock (or a different
protocol) is the only answer: when the **host or the GPU** will read guest memory *later, on its
own schedule*. We cannot copy on the GPU's behalf. That is not a gap in the discipline — it is
the reason the taxonomy has more than one class.

---

## 1. The four classes, and which even need an argument

`guest_memory_lock.md` §3.1 already excludes three of the four classes **by construction**, so
GL11 applies to a very short list:

| class | why it needs no GL11 argument |
|---|---|
| **Volatile / atomic** (completion semaphores, USERD, PTIMER) | Excluded by **who writes it** — the host or the GPU. A lock over our own reads excludes nothing; the mechanism is blind to those writers. Correctness comes from aligned atomics and the two fences (`l1_os_shell.md` §4.3). |
| **Copy-once / commit-point** (page-table pages, userspace pushbuffers) | Excluded by **rate** (GL2 refuses the ~18 kHz page-table class) **and** by something stronger than a lock: **the host GPU executes from our copy, not the guest's buffer.** A guest mutating its pushbuffer after the doorbell races *itself*, exactly as it would on real silicon. |
| **Isolate-shared** (guest-RAM slices exported to isolates) | Excluded by **GL4**, at registration, because the isolate has a **different `mm`**. ★ This is also the answer to "does uffd trap an isolate's writes?" — we never lock isolate-shared memory, so the question does not arise for the lock. It remains a live question for anything *else* that assumes it can observe isolate writes. |
| **Lock path** | ★ **The only class that needs an argument.** Two members, below. |

---

## 2. Region: the **GSP command queue** (guest → GSP) — the only settled `LockPath` member

**What it is.** A ring in guest RAM that the guest driver writes commands into and we consume.
Geometry (base, size, element stride) is **fixed at initialisation** and is *ours* — it comes
from the boot handshake, not from any individual command. **[src]** the transport shape is
described in `crates/kayfabe-gsp/src/lib.rs`'s module docs (the "single shared status queue
with its strictly monotonic seqNum ring"), and the C's implementation is
`C: src/qemu/nvkvm_gpu_emul.c` around the `msgq` handling.

### 2.1 The GL11 argument — **a lock is NOT required**

Every step can be made to satisfy §0's invariant:

1. **Read the producer index once.** It is a single aligned word. A racing update means we
   observe an older index and process fewer commands this round — which is indistinguishable
   from the guest having rung the doorbell a moment later. **Not a correctness problem.**
2. **Copy the element once**, bounded by the **fixed stride from the boot-time geometry**,
   never by a length inside the element. This is the load-bearing sentence: the copy's extent
   is derived from state the guest cannot rewrite mid-parse.
3. **Parse and validate entirely from the copy.** Every handle, length, offset and class id used
   after validation is read from private memory. The guest may rewrite the ring slot freely
   afterwards; we never look at it again.
4. **A payload that does not fit the element** is either (a) refused by the bounds check on the
   copy, or (b) fetched as a **second, separately bounded** copy whose length came from the
   *first* copy. The payload's *content* is untrusted in either case, so a race changes only
   which untrusted bytes we got — see §0 item 1.

⇒ **Conclusion: `LockPath` is not justified for the command queue.** The requirement in
`guest_memory_lock.md` §1.1 — *"we read a descriptor, resolve it, then issue a host operation
derived from it, and copy-once alone lets the guest rewrite the descriptor in between"* — is
satisfied by resolving from the **copy**. It only bites if some step resolves from the copy and
then **re-reads guest memory**, which §0's invariant forbids by construction.

### 2.2 ★ What would overturn this, stated so it is falsifiable

Register `LockPath` **only** if the GSP implementation turns up a command where **all** of:

- a host RM verb that is **not undoable** (`l1_os_shell.md` §7.8) is issued mid-parse, **and**
- a **later** step of the same command must read guest memory again, **and**
- that later read cannot be hoisted into the first copy (e.g. its extent is genuinely unknown
  until the host verb returns).

**[inferred]** I do not currently believe such a command exists in the boot path, but
`kayfabe-gsp` is unwritten, so this is a prediction and not a measurement. **The GSP milestone
owes this file a yes/no.**

### 2.3 The half of the row that is *definitionally* not lockable

`guest_memory_lock.md` §3.3 item 1 already splits it: the **message/status** queue (GSP→guest)
is written by **us**. There is nothing to exclude, and a read-only region would trap **our own**
writes. No argument needed — it is not a candidate.

---

## 3. Region: **instance blocks** — candidate, and the open question dominates

`guest_memory_lock.md` §3.3 item 2 records that whether an instance block reaches us as a
guest-RAM write or through the emulated **BAR2 aperture** decides which mechanism even applies:
a BAR2 window write is already trapped by the VMM's own path.

**GL11 status: NO ARGUMENT CAN BE WRITTEN YET**, and under GL11 that means **it must not be
registered `LockPath`** — the rule is a written argument, and "we do not know the aperture" is
not one. **[open]** The settling experiment is the bench one already named in §3.3.

**★ Note the likely outcome:** if instance-block writes arrive via BAR2, they are **trapped
writes**, not guest-RAM writes — the region lock is irrelevant and the ordinary MMIO path
already serialises them. That would leave §2 as the *only* candidate in the entire design, and
if §2.1 holds, **the lock has no members at all.**

---

## 4. Where this leaves the mechanism decision

The owner's decision (task #48) is **uffd on both architectures** — one mechanism, arm64 kept,
cost is one sysctl **or** one udev rule (probe both at runtime; refuse loudly only if neither
works).

That decision stands and should stay, but this file changes its **weight**: if §2.1 survives the
GSP build and §3 resolves to BAR2, then the region lock is a **capability we keep for a case we
have not yet met**, not a load-bearing part of the data plane. That is a good position — the
mechanism is cheap when unarmed (**[measured]** 0 cost unarmed; 6.5 µs per lock cycle,
+27 ns/page) — but it should be *stated*, so nobody later assumes the lock is protecting
something it never had a member for.

**Reciprocally:** if the GSP build turns up the §2.2 shape, this file must say so loudly, and
the lock becomes load-bearing on the hottest path in the design.

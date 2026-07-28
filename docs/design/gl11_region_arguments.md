# GL11 — the per-region "no lock at all" arguments

> **GL11 (normative, `guest_memory_lock.md` §3.4).** A region is registered `LockPath` only
> with a **written argument** that the caller cannot be restructured to revalidate instead.
>
> This file is that written argument, per region. A region with no entry here **must not be
> registered `LockPath`**.
>
> ★★ **Status (2026-07-28): SETTLED, and the lock has NO MEMBERS.** This file used to say
> *"DRAFT — … written against the protocol, not against running code, because `kayfabe-gsp`
> is still a 34-line skeleton"*. The crate is now built (~3,550 lines, S0–S5, `f2055bf`),
> and it implements §2.1's shape **literally** — see §2.2a. Both candidate regions are
> non-members, no `LockPath` registration exists anywhere in `crates/`, and `lock_region`
> has already left the `Vmm` trait (decision #41). **The mechanism choice therefore governs
> an empty set**, which is why the uffd-vs-permanent-RO disagreement in §4 is not urgent and
> should not be settled by guessing — see §4.

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

~~**[inferred]** I do not currently believe such a command exists in the boot path, but
`kayfabe-gsp` is unwritten, so this is a prediction and not a measurement. **The GSP milestone
owes this file a yes/no.**~~

### 2.2a ★★ The GSP milestone's answer: **NO** — and it is structural, not incidental

**[measured, 2026-07-28]** The debt in §2.2 is paid. `kayfabe-gsp` was built after this file was
written (S0–S5, ~3,550 lines, `f2055bf`) and implements §2.1's four steps literally:

- `boot.rs::GspFsm::service_command_queue` — the producer index is read **once**
  (`read_u32(cmd_write_ptr_off)`); the first copy is bounded by `geom.element_size()`, i.e. the
  **boot-time geometry**, never by a length inside the element; `peek_len` derives the extent
  **from the copy**, bounded by `element_size_max`.
- `boot.rs::GspFsm::read_run` — the second, separately-bounded copy. Its own doc says the extent
  comes from the first copy's `rpc.length` and is bounded by `queueElementSizeMax` before any of
  it is read — *"the shape `gl11_region_arguments.md` §2.1 item 4 permits verbatim"*.
- `element.rs::decode_message` takes `run: &[u8]` — a **private buffer** — and `.to_vec()`s the
  payload. Guest RAM is never re-read after validation.

★ **The decisive part is that §2.2's overturning shape is *unrepresentable through the port*,
not merely absent.** The host-verb seam is
`boot.rs::CommandPolicy::respond(&mut self, cmd: &RpcCommand) -> Option<Reply>`, and it takes
**no `GuestRam` parameter at all**. It therefore cannot re-read guest memory even if a future
command wanted to — the third conjunct of §2.2 cannot be satisfied without changing the port's
signature, which is a visible, reviewable act rather than a silent regression.

⇒ **§2.1 is confirmed against running code.** `LockPath` is not justified for the command queue.

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

~~The owner's decision (task #48) is **uffd on both architectures** — one mechanism, arm64 kept,
cost is one sysctl **or** one udev rule (probe both at runtime; refuse loudly only if neither
works).~~
*(Citation correction: that decision is **#49**, not #48. Decision #48 in `l1_os_shell.md` §12
is "M2-c builds against the KVM-direct harness". The mis-citation is part of why this standing
looked more contested than it was.)*

> ### ★★ CONTRADICTED THE SAME DAY — reported, NOT resolved (2026-07-27, doc audit)
>
> Three documents written on 2026-07-27 give **three different standings** for this mechanism,
> and nothing in the code settles it, because none of it is built:
>
> | doc | standing |
> |---|---|
> | **this file** (§4) | *"uffd on both architectures — one mechanism, arm64 kept … That decision stands and should stay"* |
> | `../reference/region_lock_mechanism_study.md` §4 | uffd (row **A**) is *"the **displaced** baseline — best mechanism, second-best deployment"*; the recommendation is row **B**, permanent-RO + blocking handler, **`✘ unsound` on arm64** |
> | `portability_arm64.md` | describes the standing as *"GL13 … **refuses the capability on arm64**"* |
>
> **"arm64 kept" and "the capability is refused on arm64" cannot both hold**, and they turn on
> which mechanism wins: uffd is retry-based and survives arm64; the recommended permanent-RO row
> is emulate-based and `region_lock_mechanism_study.md` §7.5 shows arm64 cannot emulate ISV=0
> stores — QEMU injects a guest abort instead. So the arm64 answer is **downstream of** the
> mechanism choice, and this section asserts the old choice.
>
> **The measured numbers in the paragraph below are row A's** (uffd: 0 unarmed, 6.5 µs/cycle,
> +27 ns/page). Row B's are different in kind — **0 µs per lock cycle** (the memslot never
> changes) but **55.6 µs per guest write**, ceiling ~17 973 writes/s. Quoting the uffd figures
> under the current recommendation would misprice it by orders of magnitude in both directions.
>
> **This is an owner decision, not a documentation fix.** ★ Note that §3's conclusion — *"the
> lock may have no members at all"* — makes it a cheaper call than it looks: if §2.1 survives
> the GSP build, the mechanism choice governs a capability with no current members.
>
> ### ★★ RESOLVED-AS-EMPTY (2026-07-28) — the decision is DEFERRED, not made
>
> §2.1 **did** survive the GSP build (§2.2a, measured against the built crate). Combined with
> §3, the position is now:
>
> | | |
> |---|---|
> | `LockPath` registrations anywhere in `crates/` | **zero** (`grep -c 'LockPath\|RegionLock\|RegionAccess\|PageClass'` = 0) |
> | uffd code anywhere | **none** — there is no `uffd_unsafe.rs` in `kayfabe-linux-raw` |
> | `lock_region`/`unlock_region` on the `Vmm` trait | **removed** (decision #41); only `CoreEvent::LockedRegionFault` survives, and that is *delivery*, not arming |
> | §2 command queue | **not a member** — §2.2a |
> | §3 instance blocks | **not a member** — no argument can be written, and GL11 says that means it must not be registered |
>
> ⇒ **The mechanism disagreement is real but not live: it governs an empty set.** The correct
> action is therefore to *decide nothing yet*. Picking uffd or permanent-RO now would be
> choosing between two implementations of a capability with no members, using measurements
> (§4's uffd figures vs row B's 55.6 µs/write) that are not comparable and not currently
> load-bearing.
>
> **What must happen instead:** the day a region genuinely needs `LockPath`, its GL11 argument
> gets written *first*, and the mechanism is chosen against **that member's** access pattern —
> at which point the arm64 question (`portability_arm64.md` GL13) also has a concrete cost to
> weigh rather than a hypothetical one. Until then this section is history, not a standing
> decision, and **nothing should cite §4 as though a mechanism had been chosen.**

~~That decision stands and should stay,~~ The rest of this section is retained as written; but this file changes its **weight**: if §2.1 survives the
GSP build and §3 resolves to BAR2, then the region lock is a **capability we keep for a case we
have not yet met**, not a load-bearing part of the data plane. That is a good position — the
mechanism is cheap when unarmed (**[measured]** 0 cost unarmed; 6.5 µs per lock cycle,
+27 ns/page) — but it should be *stated*, so nobody later assumes the lock is protecting
something it never had a member for.

**Reciprocally:** if the GSP build turns up the §2.2 shape, this file must say so loudly, and
the lock becomes load-bearing on the hottest path in the design.

# The guest-RAM crossing — how guest pages reach the host GPU

**Status:** design sketch, 2026-08-10. The **long pole** for userspace-channel passthrough
(`mode2_channel_ownership_split.md`). Written in this repo because it continues the pages here; it
belongs in `kayfabe`'s design set and should move with them.

⚠ **What is decided and what is not is marked throughout.** Sections marked ⊘ OPEN need an owner
ruling; everything else follows from rulings already made.

---

## 1. The problem, measured

To run a guest **userspace** channel by passthrough, the host GPU must fetch the guest's own
pushbuffer and ring, and DMA its release semaphore back — so those guest pages must be **pinned into
the host GPU's address space at the guest's own VAs**.

**Today guest RAM has never crossed into the isolate at all**, and it is three gaps, not one:

1. ⊘ **The isolate wire protocol has no verb naming guest RAM, and no fd-IN path.** The only
   descriptor-carrying message is a *reply* (isolate→VMM); every other reply is read with an fd
   allowance of **zero**.
2. ⊘ **`Vmm::export_ram` is built on both backends and has zero production callers** — only tests.
3. ⊘⊘ **On the bench's own QEMU backend it could not work even with the other two.** `export_ram`
   searches only the reservations *kayfabe installed*; the guest's ring lives in the **machine's own
   RAM**, reachable solely through a trait that exposes **no host pointer and no fd — only copies**.
   The KVM backend already has the right shape (a `GuestWindow` over a real sealed memfd); it is not
   what the bench boots.

⇒ **This is a VMM-adapter increment before it is an RM one**, and that ordering is the single most
useful fact on this page.

---

## 2. What the guest RAM must be

**Decided (follows from the above):** guest RAM must be a **shared, fd-backed** block — QEMU
`memory-backend-memfd,share=on` — so the VMM holds a descriptor it can hand down. The C reached the
same requirement and found the largest fd-backed RAMBlock at realize time.

⇒ Two ways to get there, and the choice is an ⊘ **OPEN** one:
- **(A) Launch-time.** Require `memory-backend-memfd,share=on` and take the descriptor. **Measured
  2026-08-10 on a real bench:** with the flag, fd 14, **2 GiB, `rw-s`, openable from another
  process**, containing `Linux version` / `nvkvm-guest` / `systemd` / `nvidia` — **live guest
  memory**. Without it, no such fd and no `rw-s` mapping ≥ 1 GiB at all. Both configurations print a
  **byte-identical** `memory plane realized` line, so the flag is **observationally neutral**;
  default stays off. ★ And it is *free*, not merely faster, for a reason worth stating: a memfd is
  pathless but is an **open fd in the shim's own process**, reachable via `/proc/self/fd`.
- **(B) Adapter capability.** Expose *(RAMBlock fd, offset)* for a machine RAM region. No deployment
  constraint, and it is what ships for any VMM whose command line we do not control.

⊘⊘ **CORRECTION (2026-08-10) — an earlier revision of this page said "have the shim adopt the memfd
as one of its own windows." That is STRUCTURALLY IMPOSSIBLE, not merely unbuilt.**
`kayfabe-vmm-qemu/src/lib.rs:1173-1181` refuses guest DRAM on **two independent grounds** before it
ever looks at a backing: it is *"not inside any realized BAR"*, and — verbatim —

> *"★★★ The reservation BAR must be one the hypervisor does **NOT** back. **This is the whole §1.5
> safety argument, asked rather than assumed.**"*

⇒ Adopting guest RAM as a window would require **deleting that check**, i.e. deleting the
memslot-safety argument. **The crossing needs a concept that is not a window.** ⚠ Same family as
[[same-class-id-opposite-directions]]: a prescription that points at removing a boundary in order to
enable a capability.

⊘ Two smaller corrections to the same revision: *"the KVM backend already has the right shape"*
invites porting a shape across — the two `export_ram` bodies are **the same code**, ~85 % identical;
the difference is **ownership**, not shape. And *"every reply is read with an fd allowance of zero"*
reads as "flip a 0 to a 1" — in fact there is **no `recvmsg` reader on the request path at all**.

⚠ **A latent bug found before the first caller could arm it:** `export_ram` and `register_backing`
share **one `exports` Vec and one token index space** on *both* backends, so a `RamHandle.token` is a
valid `HostRegion.id` and would `MAP_FIXED` **guest RAM into a guest window**. Inert only because
`export_ram` has no callers. **Fix it before wiring the first one.**

---

## 3. What crosses, and the rule that governs it

★★★★ **OWNER RULE: no raw guest pointer ever reaches the isolate.** The guest must have **no direct
channel** to it. What crosses is always a description **we derived**, never guest bytes forwarded
verbatim.

⇒ That is exactly why the **control** plane is parsed and the **data** plane is not:

| plane | contents | treatment |
|---|---|---|
| **control** — ioctl / RPC structs | pointers, handles, lengths, class params | ★ **parse and validate.** This is where we learn what to map, and it is the only place a guest number becomes an action |
| **data** — rings, pushbuffers, USERD, semaphores | guest work | ⊘ **never parsed.** Mapped, then left alone |

★★★ **And tracking at submit time is redundant with tracking at map time.** The guest can only name
VAs that exist in its VAS, and they get there two ways — **bind-time ioctls/RPCs we serve**, and
**observed page-table writes**. Neither is ring parsing. ⇒ A VA we never mapped must be refused **at
map time, by name**; it must not be discovered at ring time. The C's per-doorbell VAS sweep was
**reverse-resolution** forced on it by not having a complete ioctl-side model — and its own
measurement of the cost is brutal: **91 932 of 91 960 walks backed nothing, ~0.1 tok/s.**

---

## 4. The crossing, step by step

1. **VMM** parses a control-plane request and derives a set of `(gpa, len)` runs for a `Proc` — from
   the address table it already maintains, **forward-populated**.
2. **VMM** validates each run: it is RAM (not MMIO), it is inside the guest's memory map, and it
   belongs to a mapping **this `Proc`** legitimately holds. ⊘ A failure here is a **named refusal at
   map time**, never a clamp and never a silent skip.
3. **VMM → isolate**: a new wire request carrying the guest-RAM descriptor plus the validated
   `(offset, length)` runs. Needs an **fd-IN allowance** in the frame reader — the protocol has only
   ever passed descriptors the other way.
4. **Isolate** maps the runs into a reservation **at the GPA-ordered layout**, because
   `OS_DESCRIPTOR` describes **one contiguous HOST VA range** while the guest's GPAs need not be
   contiguous or ascending. ⇒ **One `OS_DESCRIPTOR` + `map_dma` per contiguous GPA run.**
5. **Isolate** pins each run (`OS_DESCRIPTOR`, flags `NONCONTIG | LOCATION_PCI | COHERENCY_CACHED |
   MAPPING_NO_MAP`) and `map_dma`s it **FIXED at the guest VA**.
6. ★ **Assert placement**, per run — *"placed as asked"* — and refuse by name otherwise. This is
   where the alignment question gets answered for real, rather than in a standalone probe.

⚠ **Known hazards to build in from the start**, all measured in the C:
- **A re-created guest mapping silently re-points the same VA at a new GPA.** Requires VA→GPA
  staleness detection: free the old pin, re-pin. Without it, host completions land in dead memory
  *and* corrupt whoever now owns those pages.
- **Dedup keys must be `(client, VA)`, never VA alone** — host VASpaces are per-client, and two
  guest processes are handed **identical RM handle values**.
- **A run's key must be per-aligned-chunk, not per-run-start** — guest mappings *grow*, and a
  run-start key leaves a residency hole exactly at the old run end.
- **Mark-seen only on success**, or a failed backing is remembered as done forever.
- **Fixed-size tables degrade pathologically** rather than failing: the C's 128-entry map table,
  once full, silently re-mapped every VA every doorbell → leaked host objects → host VAS exhaustion.

---

## 5. ⊘ OPEN — how much of guest RAM one isolate may reach

Isolates are **per-`Proc`**. A single descriptor for the whole guest RAM block lets isolate *A* map
pages belonging to guest process *B*. An fd cannot be range-restricted, so there are three shapes:

- **(i) One descriptor per isolate, whole block.** Simplest, zero-copy, and the isolate is our own
  cap-dropped code — but it makes the per-`Proc` isolate boundary **advisory rather than enforced**
  for guest memory.
- **(ii) Per-run descriptors.** Enforced, and it needs a mechanism an fd does not natively provide.
- **(iii) One descriptor, plus the VMM as the only party that computes offsets** — the isolate maps
  only what it is told, and is trusted not to map more. This is (i) with the honesty that the
  boundary is a **convention**.

★ The honest framing: a compromised isolate **already drives a real GPU**, which is a larger
capability than reading guest RAM. So (i)/(iii) may be the right trade — but it should be **an
explicit ruling**, not something that falls out of "an fd cannot be split." ⊘ **Owner decision.**

---

## 6. What this does NOT need

- ⊘ **No ring parsing.** See §3.
- ⊘ **No semaphore writing.** For a userspace channel the host GPU DMAs the release into guest RAM
  itself. Writing it is forging.
- ⊘ **No blocking wait on the vCPU thread.** The doorbell path must start work and return; a
  completion is bridged asynchronously. Two independent arguments reach this — the vCPU/BQL hazard,
  and the **6 s GSP RPC budget** that covers the whole `cuCtxCreate` path.
- ⊘ **No host-fault handling, yet.** Prevention covers the static working set: everything the guest
  can name is mapped, because we mapped it. Demand migration and remap-under-a-running-channel are
  successors, not preconditions.

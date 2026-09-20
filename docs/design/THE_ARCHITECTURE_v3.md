# kayfabe v3 — the architecture after the owner's review

**STATUS: LIVE, 2026-09-20 (w819). Supersedes `THE_ARCHITECTURE_v2.md`**, which this document
keeps only as the record of what was proposed before the review corrected it.

⊘ Written from the owner's point-by-point review. Where the review corrected me, the
correction is marked **[CORRECTED]** with what I had wrong — those are the load-bearing edits,
not footnotes.

---

## 1. The shape

**One process.** The VMM (QEMU, or later any hypervisor) with the kayfabe core linked in.
No isolate children, no scratchpad process, no IPC, no sandbox plane of our own.

**Two threads besides the vCPUs:**

| thread | job |
|---|---|
| **worker** | everything a trap deferred: managed doorbells, GSP RPCs, TLB invalidates |
| **VA manager** | ★ *new.* One synchronous thread doing **all** `mmap`/`munmap` — page-table diffs, BAR windows, channel mappings. It replaces the scratchpad isolate. |

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

```
if off == DOORBELL (0x00BB0090):
      bound-check the token against the table size
      one atomic u64 read of the table word
      ├─ PASSTHROUGH → write the host doorbell INLINE. No queue, no wake, no lock. Return.
      ├─ MANAGED     → push the token on the queue, set this channel's rung bit. Return.
      │                 (queue full ⇒ set "inspect all doorbells" flag, do not enqueue)
      └─ UNKNOWN     → no-op. Return.
else:
      push onto the PRIVILEGED queue (RPC submit, TLB invalidate, …). Return.
```

★ **[CORRECTED] Passthrough and managed are different costs, and I had them merged.** A
passthrough doorbell is a table lookup and one dword, synchronously, on the vCPU —
**microseconds and a return**. Only emulated/translated doorbells queue and wake. Translated
and emulated behave identically from the vCPU's side; only passthrough is inline.

⇒ **This excludes `ioeventfd` for passthrough**, and that settles the question I analysed
earlier: an ioeventfd would buy a cheap exit and then pay for it with a thread context switch
to deliver a dword the vCPU could already have written itself.

⊘ The doorbell table is the **new** one: fixed-size, `u64` atomic words, copied — not the old
structure. A vCPU takes **one** lock in the whole path, the queue's, and holds it for a push.

### 2.1 The doorbell page is READ-ONLY, not emulated

★★★ The doorbell page is mapped into the guest as a **KVM read-only memslot** over the real
host doorbell page. Consequences, all of them deletions:

- Every **read** is hardware, with **no exit and no code** — including the microsecond counter.
  ⊘ **There is no PTIMER implementation in v3.** The register emulation, the refusal of guest
  writes to it, the counter-page install — all gone.
- Every **write** traps, and runs the logic above.
- When *we* need to ring a doorbell (passthrough inline, or translated from the worker) we
  write the host page **directly**, bypassing the read-only mapping.

⚠ Emulated channels need no doorbell written at all — there is no hardware counterpart.

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

### 4.3 A VA space is `(PDB base, aperture)`

★★★ **[CORRECTED], and it improves the fix I shipped.** I wrote that a root is needed *only*
to seed a walk and is never an identity. Wrong: a VA space **is** identified by its
page-directory base plus the aperture that base lives in (sysmem or vidmem) — that is exactly
what hardware puts in the instance block.

⇒ My `Pdb(0)` bug was **two defects I had conflated**: the key was missing its aperture half,
and zero collided with a sentinel for *absent*. With `(Pdb, Aperture)` as the key, a space
declared at framebuffer offset 0 is a perfectly good name, distinct from "not declared yet",
and `unwrap_or(Pdb(0))` never needs to exist.

⊘ **Root discovery is not a problem in v3.** `[owner]` a DMA map is *written into the PD/PT
tables*, and we get an invalidate on the three entry points; sysmem mappings go through the
host's own DMA allocation. Nothing has to reach us as an RM call — if it is in the tables we
analyse at refresh, we have it. And the PDB root is declared in **instance blocks**, which is
how bare metal itself finds it.

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

**What remains is the armed interrupt.** A guest that does not spin arms an event and sleeps.
For passthrough and translated we register that with the host through userspace calls and fire
the guest interrupt when it fires.

⚠ **The race, and whose burden it is — under investigation, and the answer changes the design:**
- If the arm names a **channel** and fires on *new work completing*, the waiter must already
  re-check for itself, and we only fire on genuine channel advance.
- If the arm names a **specific semaphore**, the race is ours: we must fire immediately when
  the thing is already complete.

⊘ This is being read out of ogkm rather than assumed. It is the one open item in this section.

---

## 6. The scrub

A **translated** channel. The guest's own RM scrubs before reuse and orders it correctly; our
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

1. **The interrupt race** (§5) — per-channel or per-semaphore, and whose burden.
2. **The synchronous-verb list** — which RM controls does the guest read the reply to
   immediately? Until that list exists, "no RM verb on a vCPU" cannot be judged.
3. **Fault delivery** — needed by managed memory *and* by letting the GPU fault. `[owner]` if
   the fault structures are shared with userspace we can map them through for passthrough; a
   translated fault needs its address translated on the way back.
4. **The crate model** — to be redrawn against §8.

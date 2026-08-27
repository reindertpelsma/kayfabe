# Property 2 — the subtraction, and why it is the owner's call

> ### STATUS — 2026-08-11 / ⊘⊘ **SUPERSEDED IN ITS EVIDENCE — NO OWNER DECISION IS OWED. READ §0.**
> ⊘ **The decision this document asks for was ALREADY PERFORMED, and measured, the day before
> this document was written.** §2's central claim is false at `b3ecda4`, and its exploit citation
> is a **pre-fix** measurement quoted without its revision.
> ★ A **different, smaller** residual survives — a different object — and it is named in §0.
> **Do not act on §4's question; it is posed about a topology the tree no longer has.**

---

## 0. ⊘⊘ CORRECTION, MEASURED 2026-08-11 at `b3ecda4` — this corrects §2, §3 and §4

### §2's central claim is FALSE at HEAD

> *"★★ And there is exactly ONE host address space per guest address space today, holding all of:
> the guest's channel; the host-backed framebuffer leaves at guest-chosen addresses; **and our own
> ring, cursor block and completion semaphore**."*

**Three separate things in that sentence are wrong**, all readable in the code:

- **TWO host address spaces per guest `Vas`, not one** — `Vas::host_vas`
  (`kayfabe-core/src/gpu.rs:171`) **and** an `ExecutorVas` minted lazily beside it
  (`kayfabe-isolate-host/src/rm.rs:2975`, one per guest range). `free` disposes of both
  (`rm.rs:3285`).
- **The isolate's ring / USERD / completion semaphore are NOT in the guest's space** —
  `ce_copy_outcome` → `executor_vas(key)` → `ce_channel(key, exec)` →
  `alloc_channel_for_isolate(vas: ExecutorVas)` → `alloc_channel_in(vas.range, …)`
  (`rm.rs:4442,4443,3812,3817`). Mapped through `exec.range` and **nothing else**, and
  `ExecutorVas` cannot be *spelled* by a caller holding a guest `Vas` (private field, one mint
  site, plus a `trybuild` case).
- **The "cursor block" is in NO GPU address space at all** — USERD goes to RM as
  `hUserdMemory[0]` and is only ever CPU-mapped. No guest VA can name it under any topology.

### ⊘ And the exploit citation is a PRE-FIX measurement quoted without its revision

§2 cites R30 arm C — *"a copy engine bound to the guest's address space read our own semaphore
payload back"*. That was measured at **`cc5d55c`**. The fix landed at **`254cf38` (2026-08-10)**;
`ae73f6b`, the audit §2's co-location row rests on, is an **ancestor** of it. The same arm was
**re-measured and REFUSED**, with hardware's own word:

```
★  R30 arm C = the guest-bound engine did NOT retire a read of 0x1_20022000
NVRM: Xid 31 … CE0 faulted @ 0x1_20022000, FAULT_PDE ACCESS_TYPE_VIRT_READ
```

re-confirmed again at `b39f95f`. ⇒ ★★★ **This is the *"a ruling's DATE is part of the citation"*
class, committed by me, against a MEASUREMENT rather than a ruling.** §3 calls `ExecutorVas`
*"the precedent that makes it tractable"*: it is **not a precedent — it is the fix**, already
applied to the object §2 names.

### ★ What actually survives — a DIFFERENT object

The residual is the **materialized guest channel's own 64 KiB ring**, via `alloc_channel` →
`alloc_channel_on` → `alloc_channel_at(vas, …, None)` → `RingSource::Ours`, `raw_map_dma`'d into
the guest's range. `executor_vas_separation.md` §6 **excludes it explicitly** (*"stays in the
guest's space by design — it is the guest's channel"*). ⇒ §3's *mechanism* sentence
(`alloc_channel_at` must change) aims at the **right call**; §2's *evidence* aims at an object
that **already moved**.

★★ **And it needs no owner decision and no new address space.** `w230`'s
`alloc_channel_over_guest_ring` (`rm.rs:3791`) builds the channel over the guest's **own** pages,
so `RingOwner::HandedIn` maps *nothing of ours* into the guest's space. It is **built**, with
**one caller — the R31 probe**. Promoting it to the doorbell path removes the residual **as a
side effect of work the execution plane needs anyway**.

★★★★★ **THE INVERSION, found 2026-08-11 by the rung-preparation pass — and it reverses an
ORDERING, not just a claim.** Property 2's *entire* residual is the materialized channel's own
64 KiB ring mapped into the guest's `host_vas`. `RingOwner::HandedIn` maps **nothing of ours**
there. ⇒ **Property 2 is DISSOLVED BY the GR execution rung, not a PREREQUISITE of it.**

⊘ This corrects `gr_execution_boundary.md` §4.1, which orders property 2 **before** opening the
route. That ordering is backwards: the route's own mechanism is the subtraction.
⚠ **Fold this into §4.1 above its ordering before any code cites that ordering** — otherwise §4.1
reads as current and sends a lane at a prerequisite that the successor supplies.

### ⊘⊘ RETRACTED — "ruling 2 does not dissolve this" was MY BAD RELAY (owner, 2026-08-11)

★ **I wrote that ruling 2 was "refuted on two code facts". It was not, and the owner caught it.**

**The error is a CONFLATION OF TWO SPACES.** Fact 1 — *"every CE copy this tree can issue is
VIRTUAL (`LAUNCH_SRC_VIRTUAL | LAUNCH_DST_VIRTUAL`, `rm.rs:1930`; the `_PHYSICAL` variants refused
by name), therefore kernel CE work needs **a** host VA space"* — is **true, and it is about the
SCRATCHPAD's space**, i.e. the owner's *managed* channel. It says **nothing** about whether the
**guest kernel channel's** VAS must exist on the host. Those are different spaces, and I relayed a
claim about one as if it settled the other.

**The owner's reasoning, which stands:** a guest kernel channel is **emulated** — we manage its
USERD / ring / pushbuffer / semaphore in fake framebuffer, and the guest kernel believes it is
driving a real GPU while it is driving us. Its VAS need not exist on the real GPU because
**(1)** the channel does not exist there either, and **(2)** we intercept the commands *and every
operand that can carry a VA*, so we translate them ourselves. Real GPU work derived from a kernel
command runs on a **separate scratchpad channel with different VAs**, which we maintain with our
own GPFIFO / ring / USERD / pushbuffer / semaphore.

⚠ **And "applied literally today it turns forwarding OFF" was not an argument either.** With no
`host_vas`, `plan_doorbell` returns `FwdFault::NoVas` and `plan_ce` returns `FwdFault::NoHostVas`
— but that is a statement about **today's wiring**, which routes kernel CE through the guest's host
VAS. A ruling that says *stop doing X* necessarily breaks code that does X. ⇒ **That is the work,
not a refutation.**

### ★ What SURVIVES from that pass — a finding, not an objection

**Fact 2 is real and worth keeping**: `map_dma_both` runs `raw_map_dma(guest_range, …)` **first**
and feeds RM's returned address into the shadow, so today **operands enter the executor space
THROUGH the guest space**. The VMM translation the ruling invokes exists in
`AddressTable::resolve`, and **no verb consumes it that way.** ⇒ The mapping path is **built the
wrong way round for the emulated axis**. That is a thing to fix.

★★ **And the undecodability objection does not reach the ruling**, for a reason worth stating: the
MME defeats every method allowlist (guest microcode whose output is commands), which kills
"decode everything" **in general** — but it bites only on graphics/compute, which under the
owner's split is **passthrough and never decoded**. ⇒ **You only have to fully intercept what you
emulate, and you emulate only the kernel's channels.** The two-axis split is what makes the
interception claim survivable.

### ⇒ THE MISSING DECLARATION (owner, 2026-08-11) — and it is measured absent

> **Every channel we present to the guest is one of two kinds** — **passthrough** (unprivileged
> userspace) or **emulated** (privileged kernel). **Every channel we allocate on the host** is one
> of two kinds — **passthrough** (unprivileged guest userspace, isolated) or **managed** (usually
> scratchpad; need not be isolated).

✔ **MEASURED 2026-08-11 — this abstraction is NOT in the Rust core:**
- `kayfabe_core::gpu::Channel` (`gpu.rs:369`) carries `id`, `key`, `gpu`, `vchid`, `vas_pdb`,
  `vas_origin` — **no kind, and no privilege axis at all.**
- The **guest-facing** axis exists only as a *derivation*: `ClientKind::{Kernel, User}` on the
  owning client, reachable via `by_pdb → ProcId`. Never on the channel.
- The **host-side** axis exists only as `RingOwner::{Ours, HandedIn}` (`rm.rs:527`) and
  `RingSource::{Ours, Guest}` (`rm.rs:681`) — both declared **without `pub`**, private to one
  file, so **the core cannot speak them**, and both describe the **ring**, not the channel.

★★★ **The cost is already paid and measured**: `forwarding_plane_owns_ce`'s `proc != SYSTEM_PROC`
term **is** the guest-facing axis. Its absence cost **12 boots** of `RmInitAdapter` `NV_ERR_TIMEOUT`,
and it was fixed by **inlining the derivation into one gate** rather than by declaring the kind.
⇒ Same shape as the GPGA region kind (`gpga_region_kinds.md`) and the dropped channel `engineType`:
**the tree derives what the guest declared.**

### ⚠ NEW, `[NOT MEASURED]` — an exposure the separation itself created

`map_dma_both` places every guest publish in the `ExecutorVas` at the **guest-chosen** VA, while
our CE ring sits there at an **RM-chosen** VA. A fixed publish colliding with our ring makes the
shadow map fail, and the verb tears the guest-side map down and returns `PlacementRefused` for a
publish that would otherwise succeed. ⇒ **A guest can locate the isolate's CE ring by binary
search over publish refusals** — in a space it can never read. **Address disclosure, not a read.**
Inferred from `rm.rs:3029-3040`; **no probe has asked hardware.**

---

## 1. The three properties, and where they stand

The graphics execution route was gated on three properties. As of `814b225`:

| | property | status |
|---|---|---|
| **1** | the operands **resolve** | ★ **DISCHARGED, MEASURED.** All three address planes armed simultaneously on a booting tree: 5 of 5 operands accounted for, 24/24 host-backed `placed_as_asked=true`. Predictions pre-registered and committed before the boot |
| **2** | our own machinery is **unreachable** in the guest's graphics address space | ⊘ **OPEN. THIS DOCUMENT.** |
| **3** | a guest-caused GPU fault is **contained** | ★ **DISCHARGED, MEASURED.** A bystander context ran **2 675 519** verified iterations across the attacker's fault: 0 errors, 0 wrong bytes, no escalation, no reboot latch |

---

## 2. ★★★ Why property 2 is not theoretical — it was MEASURED EXPLOITABLE

⊘ It was once argued as *"not exploitable today, and latent."* **That was refuted by measurement**
(R30 arm C, real GA106): a copy engine **bound to the guest's address space read our own semaphore
payload back, value for value** — a number obtainable no other way.

> **The only thing standing between a guest and the isolate's semaphore was that nobody had pointed
> an engine at it.**

★★ **And there is exactly ONE host address space per guest address space today**, holding *all* of:
the guest's channel; the host-backed framebuffer leaves at **guest-chosen** addresses; **and our own
ring, cursor block and completion semaphore.** The separation is **stated in the design docs and
violated in the placement.**

⚠ **The bill is already being paid**: every publish is currently done twice.

---

## 3. What the change actually is

**A subtraction.** Not a feature — the removal of a reachability that exists today.

- **The mechanism**: `alloc_channel_at` must change so the isolate's ring / cursor block / completion
  semaphore are **not placed in the address space the guest's graphics channel can name.**
- **Why it is architecture, not a patch**: it changes *where our own execution machinery lives*, which
  touches the isolate's address-space model — the thing the isolate exists for. ⇒ It is not a line;
  it is a decision about the shape of the plane.

★ **The precedent that makes it tractable**: the same shape has already been separated once
(`ExecutorVas`), and the framebuffer operands are already placed at guest-chosen addresses
successfully (`placed_as_asked=true`, 24/24). **The machinery to place things deliberately exists.**

---

## 4. ⇒ THE QUESTION FOR THE OWNER

**Do we separate the address spaces now, before any graphics execution is attempted?**

| | if YES — separate first | if NO — execute first, separate later |
|---|---|---|
| **cost** | an architecture change on a plane that is otherwise green; delays the first execution attempt | the first attempt happens sooner |
| **risk** | ⊘ none measured — properties 1 and 3 are already discharged without it | ⚠ **the measured read-back becomes reachable by a real workload**, not just by a probe |
| **rework** | none — the separation is a precondition either way | ★ every execution result measured before the separation is measured **in the wrong address-space topology**, and may not survive it |

★★★ **My recommendation: SEPARATE FIRST**, and the reason is not security — it is that
**a result measured in the wrong topology is a result that has to be re-measured.** This campaign has
paid that price repeatedly: the address plane was measured four separate times under configurations
that turned out not to be the shipping one. ⊘ **The security argument is real but secondary** — the
hostile-guest posture is the product's value proposition, and shipping a measured read-back into the
first execution rung would be the wrong order regardless.

⚠ **The counter-argument, stated fairly**: `CE-SUBMIT` has been 0 for the entire campaign, and
**nothing has ever executed.** An architecture change made *before* the first execution is a change
made without the one measurement that would tell us whether the topology matters in practice.
⇒ **If the owner prefers to see something execute first, that is a defensible call** — provided the
first execution rung is explicitly labelled *"measured in the pre-separation topology, to be
re-measured after."*

---

## ✔ Evidence, all measured on a real GA106 and committed
- **Property 1**: `traces/boots/w247/`, with `predictions_recorded_before_the_boot.md`.
- **Property 3**: `traces/boots/w248/`, `SCRIPT_RC=0`, 5 of 5 predictions.
- **The exploit**: R30 arm C, `executor_vas_separation.md`.
- **The co-location**: `s1_what_does_it_protect.md` §3 — one host address space per guest address
  space, everything in it.

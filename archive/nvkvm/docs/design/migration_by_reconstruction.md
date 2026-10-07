# Migration by reconstruction — the owner's design, and what I think of it

> ### STATUS — 2026-08-11 / **DESIGN-ONLY — DEFERRED BY THE OWNER, banked so it is not lost**
> Owner sketched this 2026-08-11 and explicitly deferred it: *"maybe later worth researching. will
> do in another chat."* ⊘ **Nothing here is measured. Nothing here should be built now.**
> ★ It is recorded because it supplies an **independent second argument for design work already in
> flight** (§3), and because deferring a design is not the same as losing it.

---

## 1. The owner's sketch, as stated

1. **Pause the guest** — SIGSTOP-equivalent; something similar exists for a VM, or can be done via
   ioctl.
2. **Export GPU state:**
   1. all **GPGA regions** — aperture, type (unallocated / fake FB / real GPU / DMA-to-guest, with
      location in GPGA)
   2. all **initialized channels** — type, chosen **guest token**, options, enough to recreate them
   3. ask the NVIDIA ioctl to **export the registers of any paused channel** (instruction pointer;
      presumably a list exists)
3. **Attach boot state and misc.**
4. Ship it as **vendor-specific VM migration state**, alongside what QEMU / Cloud Hypervisor
   already serialize.
5. **On load**, reconstruct: redo the allocations, recreate the channels, set the registers, let the
   VMM rebuild its own CPU-side state, and resume.

Owner's question: *"I think prob this can all be done unprivileged."*

---

## 2. ⇒ MY ASSESSMENT — sound, with two substitutions

### ⊘ Substitution 1: QUIESCE, do not snapshot in-flight engine state

**Step 2.3 is the part I doubt.** RM does save and restore channel context — that is what context
switching *is* — but it lives in **context buffers RM manages**, and I would not expect an ioctl
that hands *unprivileged userspace* the mid-execution engine state (program counter, warp state) of
a running channel. ⚠ `[NOT MEASURED — this is my expectation, and it is exactly the kind of claim
this campaign has repeatedly found backwards. Check ogkm before believing it.]`

★★★ **But you probably do not need it.** **Drain instead of snapshot**: hold new submissions, wait
for every channel to retire (`GP_GET == GP_PUT`, semaphores at their target), and then there **is**
no in-flight state — a quiesced channel is just its configuration plus the contents of memory. Both
of those you already have.

⊘ **The price, stated honestly:** you cannot migrate mid-kernel. A long-running kernel delays
migration until it finishes. **For LLM inference that is a non-issue** (kernels are milliseconds).
For a multi-minute training kernel it is a real limit — and it is the same limit the vendor
solutions have.

### ⊘ Substitution 2: this is SUSPEND/RESUME, not LIVE migration — and that is good news

Pause → copy → reconstruct is **suspend/resume**. **Live** migration additionally needs *iterative
dirty-page tracking of GPU memory while the GPU is still writing to it*, then a final short stop.
That is the genuinely hard part and it is where vendor implementations spend their effort.

⇒ **Do not let the two blur.** The sketch as written is a complete and achievable *suspend/resume*
design. Live migration is a strictly harder successor that needs a dirty-tracking mechanism nothing
here has. ★ Suspend/resume alone already buys host maintenance, rebalancing, and checkpoint/restart
— most of the operational value, at a fraction of the cost.

### One thing the sketch omits: the BYTES

§2.2.1 lists region **types**; the **contents** also have to travel. Real-GPU-memory regions must be
read back (CE copy to host, or map and read). For a 12–24 GB card that is the dominant cost of the
whole operation and it sets the pause duration. Not a flaw — just the line item that decides whether
the feature is usable.

### Unprivileged? — probably yes, and for a specific reason

Everything above is RM ioctls against **our own** objects, plus reading back **our own** memory.
Nothing requires a capability the isolate has already been measured to surrender (`CapEff=0`, R25 /
R26). ★ The one step that would have needed privilege is the in-flight register export — **and
quiescing removes it.**

---

## 3. ★★★ WHY THIS MATTERS NOW, THOUGH IT IS DEFERRED — two arguments it hands to live work

### (a) A DECLARED region kind is serializable; a DERIVED one is not

Step 2.2.1 exports each region's **type**. Today that type is
`Representability::{HostBacked, Fabricated, Untracked}` — **computed from whatever happens to be
present**, with `Fabricated` as the unguarded fall-through and `Untracked` routing to real hardware
(`gpga_region_kinds.md` §0). ⇒ **You cannot serialize a fact you re-derive at every use.** Save it
and you have written down an accident; restore it and you get whatever the new host's accidents
produce.

★ **So migration is a SECOND, INDEPENDENT argument for the declared four-kind taxonomy**
(`gpga_region_kinds.md`, `gpga_kind_is_decided_at_bind.md`) — the first being that it makes
`BackingBytes`'s hazard unrepresentable. Two unrelated requirements landing on the same design is the
strongest signal available that the design is right.

### (b) OUR INDIRECTION IS WHAT MAKES MIGRATION POSSIBLE AT ALL

Step 2.2.2 exports the **guest** token. On restore, the guest's token is unchanged; the **host**
token is new, and the translation table is re-pointed. ⇒ **The guest never learns it moved.**

★★★ **That is the whole reason this is tractable here and is not for raw vfio passthrough**, where
the guest addresses real hardware identifiers directly and there is nothing in the middle to
re-point. It is also why the vendor's mediated path can migrate and its passthrough path cannot.
⇒ **The trap-and-translate doorbell — adopted for correctness — is simultaneously the migration
primitive.** Do not optimise it away.

### (c) The two-axis channel kind splits the work asymmetrically, in our favour

Under the owner's channel taxonomy (`passthrough` / `emulated` guest-facing; `passthrough` /
`managed` host-side):

- **Managed / scratchpad channels are OURS and carry no guest-visible state.** They do not need
  faithful reconstruction — **rebuild them fresh** on the destination.
- **Only the passthrough half needs faithful reconstruction**, and it is reconstructible from
  declarations the guest itself made and we already record.

⇒ **The taxonomy roughly halves the migration surface**, and it does so for free.

---

## 4. ⚠ What must NOT happen while this is deferred

**Do not foreclose it.** The concrete discipline is one line, not a project: **as each piece is
built, record which side of the managed/passthrough line it falls on.** The managed half is ours to
relocate; the passthrough half is the pinned part. Naming that as you go is free. Retrofitting the
distinction across a built system is not.

Related: `gpga_region_kinds.md`, `gpga_kind_is_decided_at_bind.md`,
`is_passthrough_the_only_correct_route.md` §4 item 10 (*"migration: NO DESIGN DOC EXISTS IN EITHER
TREE"* — this doc is the first, and it is a sketch, not a design).

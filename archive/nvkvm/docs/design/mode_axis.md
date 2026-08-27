# The `mode` axis — and the ruling that it stays an option, not a plan

**Status:** owner ruling, 2026-08-10. Short by intent. This is a **decision record**, not a design;
nothing here is a work item.

---

## The axis

`kayfabe` may eventually carry **two front doors on one architecture**:

- **`emulated`** — today. A **stock, unpatched** guest NVIDIA driver against our device model.
- **`paravirtual`** — the C's Mode 1 shape: a small guest module forwarding RM ioctls over virtio.

★ It is a **MODE, not a second device model**, by `no_non_gsp_boot_path.md`'s own test: both would
share the isolate, the forwarding plane, the RM allowlist and the ABI tables.

---

## ★★★ THE RULING

1. **The Rust stays Mode 2.** No pivot, this week or on the current milestone path.
2. ⊘ **Building paravirtual is NOT decided.** It is an open option and nothing more.
3. ★ **The intermediate checkpoint is UNCHANGED: Mode 2 full compatibility.** Everything before it
   is untouched by this page.
4. This **moves the north star further out** — it does not move anything nearer.
5. **The only obligation it creates is: do not FORECLOSE it.**

**Why the headline survives, and is arguably strengthened.** Mode 2 remains the flagship, so *"a
stock, unpatched guest driver"* stays true. And *"both front doors, with the stock-driver one as the
default"* is a **stronger** claim than either alone — the differentiator was never *"we can only do it
the hard way"*, it is *"we can do it the hard way."*

⚠ This does not disturb the standing release commitment — **public release at Mode-1 parity**, i.e.
Mode 2 reaching Mode 1's capability set. That bar is what §3 above names, and it is unaffected.

---

## What "do not foreclose" concretely means

Four properties. Each is **cheap to preserve now and expensive to restore later**, and all four are
worth having *even if paravirtual is never built* — see the last section.

1. ★★★ **What establishes the association between a `ProcId` and a guest process?**

   ⊘⊘ **CORRECTION (2026-08-10, same day): an earlier revision of this page said "kayfabe's key is
   CR3, which is the same principal as the C's `mm`, so the seam is probably already the right
   shape." That is VOID.** `kayfabe-arch/src/lib.rs:191`, verbatim:

   > *"E0 (proven on the bench, 2026-07-19): the work-submit token identifies the target channel; the
   > core demuxes on the decoded `VChid` — **no CPU-state (CR3) read exists anywhere in the
   > design**."*

   CR3 occurs four times in the tree and **none is a key**: two describe the **GPU's** page-directory
   base metaphorically as *"the GPU's CR3"*, one is an unused field in a vCPU struct, and the fourth
   is the line above saying it is not used. `ProcId` is `ProcId(self.next_proc)` — an **internally
   minted counter**, not derived from any guest register. ⚠ The claim came from a memory dated
   **2026-06-03** that was **superseded on the bench six weeks later**, and it was cited **from its
   filename** rather than its contents.

   ⇒ **The real question, and it is sharper than the one it replaces.** The C's principal is `mm` —
   address-space identity, not tgid, *"because tgids get recycled"* and `CLONE_VM`-without-
   `CLONE_THREAD` tasks *"can already read/write each other's memory directly, so they are ONE
   security domain"* (`C: src/guest/nvkvm_session.c:28-48`). Ours is an **opaque counter**, which is
   **abstract over its derivation by construction** — arguably *better* for this axis than CR3 would
   have been. So the audit must ask not *"what is the key"* but:

   > **What ESTABLISHES the association between a `ProcId` and a guest process?**

   If that association is made at the **doorbell / `VChid` demux**, it is emulated-only and the seam
   **is** at risk. If it is made through the **RM client graph**, a forwarding path shares it.
   ⊘ Do not assume either — `proc_is_not_a_set_of_rm_clients` says a `Proc` is *not* simply a set of
   RM clients, so the answer is neither obvious nor already written down.
2. The forwarding ports (`kayfabe-fwd`, `IsolateFactory` / `Isolate` / `RmBackend`) must not **name or
   assume a device-model caller in their types**. A virtio transport must drive the same ports.
3. Nothing should assume a **single guest-visible device** — two present at once, guest picks which
   driver to load, is the intended shape.
4. `kayfabe-abi-gen` should cover the structs a **forwarding** path needs, not only the subset the
   **device model** consumes.

⊘ **Out of scope, entirely:** any paravirtual code, the guest module, the wire protocol, or changing
any port. The audit that measures 1–4 is `mode_axis_seam_audit.md`; if all four come back
ALREADY-FINE, three lines and back to GR is the correct outcome.

---

## ★ The reason this earns its keep even if paravirtual is never built

**A substrate clean enough to carry a second front door is a better substrate for the one we have.**
If the isolate, the forwarding plane or the ABI tables turn out to name a device-model caller in their
types, that is a **coupling worth knowing about on its own terms** — it means our layers are less
separable than the architecture claims. ⇒ The audit is partly a check on kayfabe's own structure,
with paravirtual as the **probe** rather than the point.

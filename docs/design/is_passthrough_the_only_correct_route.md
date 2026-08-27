# Is passthrough the only *correct* route, or just the fast one?

> ### STATUS — 2026-08-11 / **LIVE**
> **ANSWERED.** Owner's question: *"earlier in docs you gave passthrough isn't just optimization for
> the guest userspace, it's the only true correct route for several reasons — but I do not know if
> still correct."*
>
> **Answer: the reasons are real; the QUANTIFIER is not.** *"The only correct route"* is refuted — by
> the owner's own **2026-08-07** ruling, and by the C reaching `bad=0` without it. A narrower form
> survives everything thrown at it. **Read §1 before citing any reason from this family.**
>
> ⊘ Supersedes the *strong reading* of `mode2_channel_ownership_split.md` §5c (2026-08-10). That
> section's **reasons** stand; its **scope** does not.

---

## 1. ★★★★★ THE FORM THAT SURVIVES

⊘ **Not:** *"passthrough is the only correct route."*

★ **But:**

> **For a guest-userspace channel, executing the guest's own bytes at the guest's own addresses is
> correct BY CONSTRUCTION. Every alternative is correct CONDITIONALLY — and the conditions are ones
> we cannot discharge.**

| condition an alternative needs | status |
|---|---|
| our parser reaches every method that matters | ⊘ **refuted for graphics/compute** — the guest uploads **39 words of its own program** to the GPU's command processor on **8/8 live channels**, and that program's *output is commands*. A scanner cannot enumerate what runs |
| the waiter only *polls*, so degraded flush scope, dropped interrupt-arming and zero-filled timestamps go unnoticed | ◐ **true for `cup8`, false in general** — the C shipped all three degradations |
| exactly one party writes each completion word | ⚠ **a PARTIAL port structurally cannot guarantee it** — §3 row B |
| nobody looks before the host has finished | ⊘ **the C's own green run violates this** — it writes the completion *before* ringing the host doorbell, and never waits |

⇒ **Passthrough is SUFFICIENT for correctness. It is not NECESSARY.** That distinction is the whole
content of this document.

---

## 2. ⊘ WHY THE STRONG FORM IS DEAD — the owner already ruled, 2026-08-07

`../../nvkvm-rs/docs/design/ce_executor_tree.md:9-23` — **read verbatim by me**:

> **"The executor is chosen by where the bytes actually LIVE, not by what the guest asked for."**
> **"A different executor producing a *true end-state* is *forwarding*, not forgery."**
> ⊘ **"Forbidden, and these are the only two things forbidden: 1. Signalling completion for work that
> did not happen. 2. Landing the data where the guest cannot see it."**
> *"An earlier reading of this repo's principles treated **'the host GPU must be the one that did
> it'** as the rule; **the owner corrected that on 2026-08-07**."*

Its three steps settle the ordering: **STEP 1 RESIDENCY (correctness — always first). STEP 2
EXECUTOR (performance — only after residency is known).**

⚠ **AND THE RULING SCOPES ITSELF**, four sections down: it governs **kernel-originated copy-engine
work only**; guest *userspace* pushbuffers are passthrough and are not inspected. ⇒ It does **not**
license re-encoding userspace work.

★★★ **The synthesis, now the standing rule:**
**READ TO ENUMERATE — yes. READ TO RE-ENCODE — no.** Discovering what the guest wants by reading its
queue violates neither prohibition. Re-generating its commands is outside the ruling's scope, and is
exactly where every condition in §1 bites.

---

## 3. THE REASONS, ADJUDICATED

| # | reason | verdict |
|---|---|---|
| **A** | **Address identity** — the guest's buffers hold literal addresses, and the completion method's address field has **no aperture override in any of the three classes**, so a wrong address cannot be routed around | ★ **HOLDS** — the only reason with independent hardware confirmation (`placed_as_asked` is a *measured predicate*, green on real GA106 across five ladder rungs). ⚠ **Re-scoped**: identity is required of the **executing channel's address space**, not of the VMM's. An address passed as an *argument* is re-addressable; one the driver takes *implicitly from the calling process* is not |
| **B** | **The second-writer hazard** — the corruption that wedged the C | ⊘ **NEVER WAS AN ARGUMENT FOR PASSTHROUGH, three ways.** The victim was a **kernel** channel. The fix moved **away** from passthrough — it *removed* the host's share of the page. And the hazard was **created by *partial* passthrough**: a lagging host engine writing stale values while software completion was also live. ★★ **Full emulation has one writer; full passthrough has one writer; a HALF-PORTED SYSTEM HAS TWO — which is exactly where both trees are today** |
| **C** | **The completion payload is unrecoverable** | ◐ **HOLDS IN KIND, REFUTED IN STRENGTH.** *"Impossible"* is measurably false — the C **transcribed** the value out of the guest's bytes and shipped `bad=0`. What it could not transcribe were the *other four* literals: flush scope never modelled, interrupt-arming decoded only to print it, timestamp zero-filled. ⇒ **Restate as: re-encoding is unsound because the method space COLLIDES and the uploaded program defeats enumeration — not because a number is unknowable** |
| **D** | **Undecodability** | ★★ **HOLDS — strongest reason in the corpus.** But note what it argues: it is a **refusal**, not an endorsement — its home doc concludes *"do not open passthrough yet"*. And it is a property of **graphics/compute**, not of pushbuffers generally: the copy-engine half was answered *"decode it statefully"* |
| **E** | **Completion timing** — a re-issued copy completes at a different time, against a different engine | ★★★ **HOLDS, MEASURED, and I UNDER-WEIGHTED IT.** The C writes the completion, then rings the host doorbell ~20 lines later, and never waits anywhere on the live path. Its `bad=0` is a genuine host result *read from behind an unwaited completion* — **"a race that usually wins is not a mechanism."** It is the named suspect for the hang that is 1/3 one day and 9/9 the next on a bit-identical binary. **Under passthrough this cannot occur: the engine writes the release after the work, by construction** |
| **F** | ★ **(missed) Delineation by privilege** | **HOLDS, and it is the cheapest to state**: a page guest userspace can write to cannot carry privileged content — *the guest's own driver already decided that*. This is what makes passthrough **safe**, as distinct from what makes it **correct**. Corroborated by NVIDIA: *"Security for these channels is enforced by VMMU and IOMMU"* |

---

## 4. ★★★ THE HONEST OPPOSITE — where passthrough COSTS correctness

A one-sided argument is worthless. Ordered by severity; every row measured.

1. ★★★★★ **A host channel born over the guest's live cursor block silently ZEROES it** — all 512
   bytes, real GA106, allocation still returning success, and the driver's own reporting path is
   compiled to *no action*, so nothing can surface it. The queue imposes no ordering; **the cursor
   block does.**
2. ★★★★★ **Address identity puts our own machinery where the guest can name it — measured
   EXPLOITABLE.** A copy engine bound to the guest's space read back our own value, obtainable no
   other way. ⇒ *"the only thing standing between a guest and the isolate's semaphore was that
   nobody had pointed an engine at it."* **And it composes with reason D**: if the address space is
   the only containment surface, granting the guest identity *in the space that holds our state*
   removes it.
3. ★★★★ **TWO MEMORIES — half-passthrough is silently wrong in BOTH directions.** The host-backed
   video-memory objects are **placed correctly and blank**: the engine would read zeros where the
   guest wrote, and write where the guest cannot see. ⊘ **That is forbidden thing #2**, and it is
   **self-concealing** — a run that dereferenced a blank buffer logs identically to a correct one.
   ★ Located at one line: the check asks *"does a host object exist here"* and **never** *"are the
   guest's bytes in it."*
4. ★★★★ **Card memory cannot back a guest memory window at all** — refused by the vendor's kernel,
   which names it itself. The gate is a property TRUE only for three integrated chip variants; a
   discrete GA106 is FALSE. ⇒ **the fd→map→window mechanism is structurally unavailable for anything
   the guest thinks is video memory — 3 of the 4 bindable operands.** ⚠ The fallback's cost has
   **never been measured, in any unit, in either tree.**
5. ★★★★ **Doorbell passthrough is MEASURED INCORRECT** — the one passthrough claim that was tested
   and failed: three guest tokens, one matching no host token. Its escape needs a guest-driver
   patch, which **breaks the stock-driver thesis**. ⇒ *"we forward nothing"* is false at the
   doorbell, permanently.
6. ★★★ **Containment collapses onto one gate, and that gate is vacuous today** (the shell passes an
   empty working set). And refusing to ring is **indistinguishable from a hang** to the guest,
   whereas re-issuing gets a proper error back from the host kernel for free.
7. ★★★ **Nested virtualisation masks the entire win** — measured: no-exit reads are **not** served
   under nesting, and the shipping regime is nested. Bare metal it is real (≈49.9 vs 47.5 tokens/s).
8. ★★ **Demand migration is carved out of scope**, not solved — the thesis partly survives by
   redefining the hard case away.
9. ★★ **Second-order bills, all real**: guest ballooning must be globally disabled; the window's
   address ceiling is the host CPU's physical-address width; no I/O-MMU translation exists anywhere;
   the window is invisible to the VMM's own view, so any other device DMAing into it reads zeros.
10. ★ **Migration / suspend / resume: NO DESIGN DOC EXISTS IN EITHER TREE** — and migration is the
    documented *vendor* reason customers choose mediated sharing **over** passthrough. Pinned pages
    at fixed host addresses is exactly the shape that blocks it, and nothing has priced it.

---

## 5. ⇒ WHAT THIS CHANGES

- **Cite the §1 form, never the strong one.** The strong form is refuted, and citing it will send a
  lane at work the owner already ruled unnecessary.
- ★ **Reason B must stop being used as a passthrough argument.** Its true content — *a half-ported
  system has two writers* — argues for **finishing a migration quickly or not starting it**, and it
  is a live risk right now.
- ★★ **Reason E deserves promotion.** It is the only one about *when anyone may look*, it is
  measured, and it has a live suspect attached.
- **§4 item 3 is a prohibition violation at a line number**, and outranks the ring-routing work.

## ✔ Verified by me before committing
- `ce_executor_tree.md:9-23` **and** its scope section — read verbatim, `git log` shows no
  supersession.
- The vendor gate: the capability is set TRUE only for three integrated chip variants and FALSE
  otherwise (`g_gpu_nvoc.c:215-224`), and the mapping path refuses when false (`nv-dmabuf.c`,
  `-ENOTSUPP`) — in `ogkm-580.159.04`, the version the bench runs.
- The guest-RAM crossing **has a production caller**; the object-fd-out crossing does not.
  ⚠ **The instrument that first reported "orphaned" could not discriminate** — it returned zero for
  a verb that runs 8× per boot. That correction is banked as `a-census-zero-needs-a-known-positive`.

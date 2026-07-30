# The eight blockers — resolutions, 2026-07-29/30

> **Status: RESOLUTIONS RECORDED.** Written from a long working session in which eight
> ★★/★★★ items were raised, argued with the owner, and verified against the C artifact.
> Every claim is cited or marked as unverified. **Two of the eight are still open**; the rest
> have an answer good enough to build from.
>
> ★ This file exists because the reasoning was expensive and lived only in a conversation.
> Three of the eight resolutions **overturned a belief held by both the owner and the
> assistant**, and two overturned a design doc. Read the corrections, not just the answers.

## 0. How to read this

Each item: **the problem**, **what was believed**, **what is true**, **the resolution**, and
**what is still open**. Where a design document disagrees with the code, the disagreement is
recorded rather than reconciled — per the standing rule that a comment is a strong prior and
not a measurement.

---

## 1. ★★★ Address identity — RESOLVED, and it is a port

**Problem.** The guest's command buffers name *guest* virtual addresses. For a forwarded
submission to resolve, the host GPU's MMU must find mappings **at those same addresses**. Our
mapping port has **no address parameter** — it asks the host driver to map and returns wherever
the driver chose. (`crates/kayfabe-isolate/src/lib.rs:352`; note `unmap_gpu_va` at `:356` *does*
take one — the asymmetry is the tell.) The real backend consequently sends `flags: 0,
dma_offset: 0` and accepts the host's choice (`kayfabe-isolate-host/src/rm.rs:702-718`).

**And the suite asserts the negation** — `tests/tests/sim_14_two_process.rs:129-131` requires
two processes' identical guest VAs to get *distinct* host VAs.

**What is true.** The C calls this *"the irreducible primitive the whole data plane rests on"*
(`C: nvkvm_gpu_emul.c:7663`) and implements it with `DMA_OFFSET_FIXED_TRUE` (bit 15, `0x8000`),
where `dmaOffset` becomes **[IN]** rather than [OUT] (`C: :7668`, `:7689`).

★★ **The test encodes a wrong reading of #14.** The proven fix is per-`Vas` **host address
space** separation, *not* host **address** separation. Under address-identity two processes'
identical guest VAs map at the **same** host VA inside **different** host VASes — which is safe,
and is the only arrangement in which a forwarded pushbuffer naming guest VAs resolves at all.

**Resolution (owner).** Adopt address-identity. Two independent changes:
- **Host CPU addresses**: reserve a large region first, then `MAP_FIXED` *within it* at
  guest-derived offsets, **and only in the isolate**. Verified as exactly what the C does —
  a 128 GiB `MAP_ANONYMOUS|MAP_NORESERVE|MAP_FIXED` reservation (`C: nvkvm_isolate.c:1270`),
  then guest-derived placement inside it (`:1306-1321`), with the isolate mapping at the guest
  address on command (`C: nvkvm_isolate_handlers.c:1713`).
- **GPU virtual addresses**: the mapping port takes `at: GpuVa` and sets the fixed-offset flag.

★★★ **Standing rule, owner: NEVER `MAP_FIXED` at a guest-supplied address in the hypervisor.**
There, `MAP_FIXED` is for placing memslot backings. Honouring a guest address happens **in the
isolate, inside a pre-reserved window**, and nowhere else.

**Rewriting `sim_14`'s assertion is correct, not a weakening** — it should assert per-VAS
separation. ★ The cheapest enforcement is a type-level invariant: *every binding with a host
backing satisfies `host_va == the VA it is bound at`*, assertable at publish and over a table
walk. It will turn that green test red, **which is the point**.

**Still open:** nothing. This is a port with a known shape.

---

## 2. ★★★ Address-table population — RESOLVED, and it overturns the design doc

**What was believed.** Owner: two populate paths converging on one table, with unmap logic and
source-dependent PDB synchronisation. Assistant: the C reconstructs mappings from
`GPU_PROMOTE_CTX` and does not decode page-table entries.

**What is true** (all `C: nvkvm_gpu_emul.c`):

- ★★★ **There is no single VA→physical table in the C.** There is a 1024-entry
  `va_map[]` (`:311-316`) fed **only** by the `PROMOTE_CTX` snoop (`:2446-2472`, the sole call
  site of the recorder at `:2470`), and **everything else resolves by live-walking the guest's
  own page tables in the emulated framebuffer on every lookup** — `nvkvm_chan_translate`
  (`:5436-5500`) has **six** fallback passes. `mode2_address_table.md` §3's *"one authoritative
  per-VAS VA→GPGA table"* **does not exist in the C.** The Rust's one-table design is genuinely
  new, and better.
- ★★★ **The copy-engine path DOES decode real page-table-entry content** — valid bit, aperture,
  physical address, at every GMMU-VER2 level (`nvkvm_m2_pt_enum_pte`, `:8588-8598`;
  `nvkvm_m2_cpt_decode_page`, `:8712-8742`). The doc is **accurate, not aspirational**.
  ★★ **It works because QEMU performs the copy itself** (`memcpy` at `:6414`), so it can
  re-read the destination page afterwards. The hot-path hook only *latches* an index
  (`:8782-8811`) — decoding per write livelocked (`:8686-8690`, bench-proven).
  ★ The decode reconstructs the VA from the page's **recorded level metadata**, not from a root
  walk, because *"the guest fills a leaf page and links it under the root a separate push
  later"* (`:8681-8690`).
- ★★ **Source-dependent PDB logic is real** — four mechanisms. A UVM-sourced root qualifies for
  the capture path **unconditionally**; a GSP-sourced one only if its client is a graphics or
  user-copy client (`:8574-8586`). Root aperture is source-derived (`:2716` vs `:2755`).
  `va_map[]` **overrides** a walked entry with no staleness check (`:5442-5450`). A sticky
  per-client table is **never pruned on handle free** and is consulted first by the semaphore
  writer only (`:382-401`, `:5578-5586`) — the #12 scar.
- ★★ **There is essentially no unmap.** An invalidated entry is skipped (`:8590`); nothing is
  reclaimed. `mode2_address_table.md:224`'s *"unmap is eager for changed ranges"* is **false for
  the C**. Reclamation happens only via GPA-change re-back (sysmem, capped 64), a compute-aperture
  flush at teardown, immediate bookkeeping prunes, and a **deferred** reap fenced on the GSP
  re-handshake — a *transport* fence, not a work-completion fence (`:2240-2249`).
- ★ **The C violates its own MISS = FAULT doctrine**: `chan_translate`'s sixth pass is a **blind
  any-client walk** (`:5492-5496`), which `mode2_address_table.md:190-196` explicitly forbids.
  Its own comment calls it "last resort" and ties it to the #12 collision class.
- ★ `va_map[]` is keyed on client **with no PDB/VAS/channel discriminator** and, uniquely among
  client comparisons in that file, is **not dup-edge aware**. Aliasing *and* under-matching are
  both real; unhit only because single-process runs have one compute client.

★★★ **Three defects in our port, by comparison:**
1. `phys: dst.0` — binds the destination to itself, publishing nothing.
2. `aperture: Vidmem` hardcoded — the C reads it from PTE bits `[2:1]`.
3. ★★★ **`if !dst_is_virtual` is BACKWARDS.** #13's root cause is that the guest's copy-engine
   utility *identity-maps the whole framebuffer into its own address space at 512 MiB pages and
   issues its page-table writes as **VIRTUAL-destination** copies* (`C: :4936-4952`). The C hooks
   on the **resolved physical** regardless (`:6437`, `:6353`). **Our gate excludes exactly the
   case #13 is about.**
4. Structural: we bind at the method; the C binds at the **semaphore release**, the guest's
   commit point.

**Resolution.** Build `#93` (promote-ctx) first — explicit `{virt, phys, size, aperture}`
entries, no decoding. Then the working-set half needs: destination-page **content**, a
GMMU-VER2 decoder including the 512 MiB leaf case, `{page → pdb, vabase, level}` metadata,
latch-at-write / decode-at-release, and the virtual-destination trigger.

**Still open:** ★★ **where the destination-page content comes from.** The C gets it free by
emulating the copy. If our core does not emulate it, the payload must be handed over or
read back. **That is a port-architecture question, not a detail.**

---

## 3. ~~★★★ #13's likely root cause — a one-line un-propagated fix~~ **REFUTED 2026-07-30**

> ★★★ **This section's hypothesis is DEAD. Do not design around it.** Measured on the bench
> 2026-07-30, after this file was written:
>
> - **`comp=1 runs=0` is not a failure signature.** The *passing* run has **more** of them
>   (40 vs 38) than the hanging one.
> - **There is no `root=SYS` in any log ever taken on this bench** — 12 × `root=FB`. Every
>   UVM root here is framebuffer-rooted, so the "un-propagated" hardcoded `false` already
>   **is** the resolved value and changing it is a provable no-op on this workload.
> - ★ That also contradicts the C's own comment that *"the UVM root is typically in
>   SYSMEM"*. On GA106 + 580.159.04, measured, it never is. One more comment that is not a
>   measurement.
>
> ★★★ **And `cup8_iter` is not an oracle.** The bit-for-bit identical binary that scored
> 1 PASS / 2 HANG yesterday ran **9/9 green** today (3/3 at the same commit, 5/5 at HEAD,
> one clean `ITERS=16`) — ~4e-6 against yesterday's rate. The rig is healthy (`mp14` still
> reproduces #14 exactly), so the variable is environmental and unidentified. **#13's honest
> status is "not reproducible on demand, root cause unknown"** — neither resolved nor
> reliably broken. A green `cup8_iter` is not evidence that anything was fixed.
>
> ★★ **What survives, and it is the useful half.** The owner's four-way diagnostic
> (below) came back **"the faulting address was NOT in our table"** in **5/5** hangs —
> zero hits at exact, 2 MiB *and* 512 MiB granularity, and not instrument blindness (the
> same logs print backing lines for neighbouring addresses in the same format). So #13 is a
> **capture gap, not a propagation gap.** The conclusion this section reached was right; the
> mechanism it proposed was wrong. Capture is what `#102`/`#93` and the operand split
> address.
>
> ★ **Unexplained lead, recorded not built to:** a second, never-enumerated 256 MiB-aligned
> region at exactly `backed_span_base − 0x10400000`, into which three of the four GRAPHICS
> faults land a bare `+0x9000`/`+0xa000`. Best guess is CUDA's kernel local-memory backing
> store, grown at the first N=2048 launch — which would explain "ITER 3 specifically". It is
> a hypothesis with no confirmation. The shape worth designing for is the general one:
> *regions the guest touches that we never enumerated.*

### The refuted hypothesis, kept for the record

Two calls thirty lines apart:

```c
// :8874  populate_cvas — FIXED as M5.36
/* pass the resolved root aperture — UVM-managed roots are sys-rooted; the
 * previously-hardcoded false mis-walked them (enumerated 0 leaves) even when found. */
nvkvm_m2_pt_enum(s, pdb, root_sys, 0, 0, &a, &budget);

// :8836  enum_gr_sysmem — THE MAIN DISCOVERY SWEEP, NOT FIXED
nvkvm_m2_pt_enum(s, pdb, false, 0, 0, &a, &budget);
```

The unfixed one is the sweep that walks **compute** VASes and records their table pages for the
copy-engine trigger (`:8833-8835`). Its predicate trusts **UVM-managed roots unconditionally**
(`:8574-8586`) — and UVM roots are precisely the ones that can be **sysmem-rooted**
(`:2749-2790` is the only path deriving aperture from flags).

⇒ zero leaves enumerated → nothing backed → **host fault one page past the last-backed leaf**,
which is the #13 symptom **in the C's own words** (`:4936-4952`: *"…the host CE FAULT_PDE'd one
page past the last-backed leaf (#13, Xid 31)"*). Measured 2026-07-29: `cup8_iter` ITER 3 hangs
with `Xid 31 FAULT_PDE ACCESS_TYPE_VIRT_WRITE` at both `fc4164d` (1/3 pass) and `862c7c2` (0/3).

★ **INFERRED, not measured.** The decisive test is already instrumented: the log line at
`:8838-8841` prints `comp=%d runs=%d ... backed=%d`. A compute VAS reporting `runs=0` is the
smoking gun. **Do not patch before measuring** — a fix landing without the failing measurement
is exactly how the "5/5 lucky sample" record was created.

★★ Owner's diagnostic, adopted, and sharper than the original three-way split: **first ask
whether the faulting address was in our table at all.** If **no** → we never recorded the
mapping (capture gap). If **yes** → we recorded it and failed to propagate to the host. Only
the second case needs the never-mapped / stale / wrong-VAS split.

---

## 4. ★★★ What a process is — RESOLVED

**Owner's definition, adopted:** *a process is everything sharing one address space.* Threads
sharing an address space are one process. **One process = one address space = one isolate.**

**Why the C breaks.** It keys per-process tracking on the **RM client**, and the guest's
unified-memory driver has **one client for the whole module load**. Measured on hardware: two
concurrent processes **share one duplicated client**, so it lands in both process records, the
lookup collapses them, and cleanup unlinks only the first. ★★ **The acceptance signal for that
feature was measuring the aliased mapping** — its only consumer keys on exactly that client, so
it could not have failed.

**And the same field breaks on Windows**: the kernel-vs-user classification keys on a sentinel
process id that exists only in the driver's UNIX build, so on Windows every kernel client
classifies as *user* and becomes eligible for the merge that keeps processes apart — **every
process plus the guest kernel in one isolate.** Fixed 2026-07-29 (`cca2cc7`) as a refusal; the
real Windows rule is unknown and deliberately not invented.

**Resolution.** Key on the address space. Candidates for the concrete key: the *originating*
client, `(client, PDB)`, or `(client, vChid)`.

**Still open:** ★ a small residue — client-level allocations happen before any address space
exists, so the key is not yet determinable in that window. The C keys those on the originating
client. A detail, not a blocker.

---

## 5. ★★★ Layer 1 — RESOLVED, and it is smaller than the document implies

**What is true.** `core_security_threat_model.md:132-139` says Layer 1 *"is built"*. It is not:
the trait method was removed with an explicit *"zero core call sites"*, nothing replaced it, and
three surviving documents disagree about whether the mechanism is userfaultfd or a
permanently-read-only mapping. The system runs on Layer 2 alone — which the same threat model
says is **not a security property**.

**Resolution (owner).** Userfaultfd is dead — skip it. Layer 1 is:

> ★★★ **A read-only *reference* into guest memory is invalid.** Copy the value out, then
> operate on the copy.

- **Bulk payloads** — copy once, validate the copy, never re-read. This is already decision
  #43; the work is *enforcing* it, not deciding it.
- **Single scalars** (a token, a ring pointer, a semaphore) — **copy by value before
  processing**. Small integers are cheap to copy and the guest may rewrite the original at any
  instant. Holding a reference and reading it twice is the bug.

⇒ No lock mechanism, no userfaultfd, no read-only mapping. The three disagreeing documents get
struck and the rule becomes gateable.

**Still open:** nothing conceptually. Enforcement is a gate plus an audit of the double-read
sites.

---

## 6. ★★ The forwarding allowlist — RESOLVED, it is a port

**What was believed (assistant, wrongly):** that deciding what belongs on an allowlist required
knowledge that would take months to acquire.

**What is true.** It already exists, derived from gVisor's nvproxy and validated in Mode 1
against 22 real applications at host parity:
- `C: src/qemu/nvkvm_ctrl_allowlist.h` — **165 control-command entries**
- `C: src/qemu/nvkvm_fe_alloc_allowlist.h` — **112 allocation-class entries**
- plus a default-deny UVM schema (`C: nvkvm_isolate_handlers.c:516`)

Recorded status: *"nvproxy-parity security model COMPLETE across all four gate categories."*

**Current Rust state:** default-**allow**; the only real Case-2 set is **two constants in the
mock crate**; class and parameter bytes pass through unsanitised.

**Resolution.** Port the lists. ★ Carry across carefully: they were built for Mode 1, where the
guest sends real ioctls; Mode 2's guest sends GSP RPCs, so the *transport* differs even though
the command space is the same. Graphics is the known exception.

---

## 7. ★★ Handle identity — RESOLVED, and the owner's fix is better than the gate

**The hazard.** RM handles are 32-bit values minted **per client** from a common base, so two
isolates independently mint the **same numeric values** meaning **different objects**. If our
bookkeeping ever carries a handle from one isolate's context into a call issued through
another's, RM resolves it as that isolate's unrelated object. ★ **This is our bug shape, not
NVIDIA's** — the per-client namespace is correct and normal.

**What actually contains it:** every request stamps the isolate's own client, so a stray value
can only resolve *inside that client*. Structural and good.

**What was claimed to contain it:** a gate that refuses foreign handles before executing a verb.
★★ But a documented bring-up escape hatch **skips the gate by design**, and that is what the
committed hardware ladder program uses **for every verb**. And the one two-isolate test runs
against the fixture, so it measures the *mock's* numbering scheme, not RM's.

★★★ **Resolution (owner), and it is better:** stop passing raw handle values in the core. Pass a
representable object carrying **which isolate the handle came from**, so one logical object has
**different numbers per isolate** and a cross-isolate value is a **type error, not a runtime
check**. Mode 1's per-isolate handle table is the precedent. ★ This also **fixes the escape
hatch for free** — a bypass path cannot bypass a type. **Requirement:** the type must be the
*only* way to name a handle, or it degrades to advisory.

**Cheap experiment still worth running:** the two-isolate test against a **real** driver,
asserting the raw values *do* collide — turning a fixture property into a measured fact.

---

## 8. ★★ Sharing guest memory with an isolate — MECHANISM FOUND, POLICY OPEN

**What is true** (this overturned both parties' beliefs):

- ★★★ **Mode 1's isolate never receives guest RAM at all** — it cannot; Mode 1 boots without a
  memfd backing. Instead: *"every continuous physical page range gets its own memfd — independent
  permissions, independent lifetime, independently `munmap`-able"* (`C: PLAN.md:77-78`). QEMU
  mints one sized exactly to the request (`C: nvkvm_handle.c:202-234`), hands it over by
  `SCM_RIGHTS`, and commands one mapping at a time. Granularity for guest CPU memory is **per
  4 KiB page**, demand-faulted. **Revocation exists** (`ISOLATE_CMD_MUNMAP`).
- **Mode 2 uses the identical plumbing** (`C: nvkvm_gpu_emul.c:6634-6638` says so) — only the
  *object* differs: one whole-RAM memfd instead of N per-range ones.
- ★★ **Mode 2's GPU-visible handover is already per-region** — discovered runs are pinned
  individually. The whole-RAM mapping is only the **CPU address-space substrate** the descriptor
  pointer must live in.
- **Why they differ — recorded, and it is guest cooperation**: *"Mode-1 only works via the guest
  module's cooperative shared GPA-window; the stock Mode-2 driver allocates in ordinary guest
  RAM"* (`C: commit 62ad21d`). ★ But that establishes only *"the stub must reach arbitrary
  guest-physical addresses"* — **not** *"the mapping cannot be narrowed."* The only text on
  whole-vs-slice says **simplicity**, and its *"matching Mode-1"* justification is **factually
  wrong**. **Nobody ever weighed and rejected per-slice mapping.**
- ★ The assistant's hypothesis (*"DMA targets are unknown in advance"*) is **refuted** — targets
  are discovered at runtime by the page-table sweep.
- **The seccomp attempt the owner remembered is real** — `install_isolate_mapping`, alive **33
  hours** (2026-05-25→26). It trapped **`ioctl(KVM_SET_USER_MEMORY_REGION)`**, *not* `mmap`;
  `ALLOW_IF(SYS_mmap)` was **unconditional in the same filter**. It died because *"KVM enforces
  `kvm->mm == current->mm` strictly … architecturally unworkable."* ★★ **A seccomp filter
  validating mmap ranges was never implemented or proposed.**

**Three security facts, none of them in any security document:**
- today's filter places **no bound** on what the stub may map, and the mmap handler does **zero**
  validation of address or length;
- **no seals anywhere** (zero `F_SEAL` in the entire corpus);
- **no revoke path in Mode 2** — the share is set once and lives until the stub dies.

★★ **Posture collision:** the recorded position is *"per-proc isolates are a blast-radius
structure, **not a security requirement**"* — while the Rust threat model's boundaries 1 and 2
require a compromised isolate to reach **only the process it serves**. Both cannot hold.

**Still open — the policy.** Chunked memfd backing was proposed and **correctly rejected by the
owner**: a chunk still contains other processes' pages, so it bounds coarsely and is an
optimisation, not a solution. The live candidate is a **supervised mmap** design (see §9).

---

## 9. The supervised-mmap sketch (owner's, under evaluation)

Give the isolate the whole-RAM descriptor, but make **mapping** the controlled operation:
a known descriptor number; `close` rejected; duplication resisted; `mmap` **range-validated**
against exactly the regions the isolate is entitled to; `munmap` tracked so that releasing a
region is **proven** before the guest's page-reclaim synchronisation is allowed to complete —
and an isolate that refuses or fails to release is **killed**.

★★ **Why this is more promising than it looks:** `mmap`'s security-relevant arguments —
descriptor, offset, length — are **all scalars in registers**. Seccomp can inspect them directly,
with no pointer dereference and therefore **no TOCTOU**. That is unusual and it is exactly what
made the earlier `ioctl` attempt unworkable but does **not** apply here.

★★★ **The fatal detail to solve: a classic BPF filter is static, and the entitled range set is
dynamic.** Options: seccomp **user-notification** (the supervisor decides per call — sound here
because the arguments are scalars, unlike the `ioctl` case), or map everything `PROT_NONE` once
and gate **`mprotect`** instead (also scalar arguments). Both put a supervisor on a hot path,
which raises the same liveness concerns as R1/F1 — and the earlier attempt's own note records the
supervisor thread wedging the stub.

★ **The `munmap`-as-proof idea is genuinely good** and composes only if the entitlement list is
updated **before** the release is acknowledged; otherwise the isolate can re-map between our
observation and the guest's reclaim.

---

## What is still open after all of this

1. **§2** — where the destination-page content comes from if the core does not emulate the copy.
2. **§8/§9** — the supervised-mmap policy, and specifically the dynamic-range mechanism.

Everything else has an answer good enough to build from. ★ Three of the eight resolutions
overturned a belief held by *both* parties, and two overturned a design document — which is the
argument for verifying against the artifact before designing.

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

### 6.1 DONE (2026-07-30) — `kayfabe_abi::capability`, and four corrections to the paragraph above

The port landed as `crates/kayfabe-abi/src/capability.rs`, enforced at the **guest ingress**
(`kayfabe_rmrpc::translate`, before the params decoder) with two new named refusals —
`BridgeRefusal::ControlNotPermitted` and `::AllocClassNotPermitted`, each carrying a `Denial`
so a census tells *"we refuse this by name"* from *"nobody has ever seen this"*. One gate, in
the place the C put its one gate, because *port the C, do not redesign it*.

**Four things the census above got wrong, all measurable:**

1. **112 allocation classes is wrong — the C has 89.** (165 controls is right.)
2. **Nine of the 165 control rows are dead.** Each has the GSS-legacy bit set or sits in the
   `NV2081_BINAPI` class, so the rule-based passthroughs answer for them whether or not a row
   exists; none has a name in nvproxy's map or in either vendored ogkm tree, so none could be
   reviewed either. They are **not carried**, and `RULE_COVERED_C_ROWS` re-checks all nine
   against the live rules so narrowing a rule turns nine silent new denials into a red test.
3. ★★★ **The Mode-1/Mode-2 transport caveat is bigger than "graphics is the exception".**
   **Six** controls this port already names are on the C's list **nowhere**: the four
   page-directory commands and the two canonical Case-2 commands
   (`GPU_PROMOTE_CTX` `0x2080012b`, `GR_GET_CTX_BUFFER_INFO` `0x20801219`). In Mode 1 the
   guest's own driver issues all six to *its* GSP and none crosses a userspace ioctl boundary
   — so a list validated against 22 applications **could not** have contained them. They are
   carried as `Origin::Mode2Rpc`, and six is a **floor**: the rest of the delta is unknown
   until a GSP boots and the refusals are read off.
4. ★★★ **"Default-deny" is true of half the command space.** `RM_GSS_LEGACY_MASK` is bit 15,
   so nvproxy's own rule — and therefore the C's, and therefore ours — passes 2³¹ commands
   with no row and no review (`gvisor/…/nvproxy/frontend.go:769-780`). Ported verbatim, and
   pinned by a test that says so, because the sentence *"an unknown control is refused"* is
   only true where that bit is clear. Narrowing it is a design decision on evidence nobody
   has yet.

**The version seam** is a field on `DriverAbiTable`: adding a driver version is a `TABLES` row
pointing at a `CapabilityTable`, which is inherit-then-add, so the row is only the delta and
**no logic crate is edited**. It bites: `NVCEB7`/`NVD1B7` exist at 580.65.06 and not at
580.65.05, and two capability-only boundaries (560.28.03, 570.86.15) exist so a 550 guest is
not silently handed a 570 guest's class set.

**Deliberately not ported:** the 23-row frontend-ioctl NR list and the 31-row UVM schema (both
gate an *ioctl* transport Mode 2 does not have — the guest's `nvidia-uvm` talks to the guest's
own `nvidia` module), and the C's 1 MiB inner-params cap (`MAX_REASSEMBLED_BODY` already binds
at 64 KiB, sixteen times tighter, so the C's number could never fire here).

**Still open:** the *host egress* (`kayfabe_fwd::classify_control`) remains default-forward. It
is fed only by tests today, it has no `kayfabe-abi` dependency by design, and a second gate
would be a second source of truth for one question. When the `Translation::Forward` arm lands,
the permit it must carry is the one the ingress already computed.

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
   ★ **§11 answers the *classification* of this question and escalates it: it is an ARCHITECTURE
   decision, not an implementation gap.** Read §11 before building anything in stage C.
2. **§8/§9** — the supervised-mmap policy, and specifically the dynamic-range mechanism.

Everything else has an answer good enough to build from. ★ Three of the eight resolutions
overturned a belief held by *both* parties, and two overturned a design document — which is the
argument for verifying against the artifact before designing.

---

## 10. ★★★ The data-plane redesign — what was BUILT (2026-07-30), and where C/D stand

`#102` + `#93` + the working-set half of the address table are **one change**. Two of its
four stages are landed on `master`; this section records the state of the other two so the
next pass executes rather than re-derives.

### Landed

| stage | commit | what |
|---|---|---|
| **A — address identity (`#102`)** | `705175b` | the mapping port takes `at: GpuVa`; the real backend sets `DMA_OFFSET_FIXED_TRUE`; `Worker::execute` refuses a downgraded placement; `AddressTable::bind` refuses a host-backed binding whose host VA is not its own VA |
| **B — the operand split** | `379f712` | classify on the **resolved physical** destination, not on `dst_is_virtual`; device-global page-table-page ownership on `Spine`; the tautological self-bind and the hardcoded `Vidmem` deleted |

★★ **Seven suite sites asserted that two processes' identical guest VAs must get
DISTINCT host VAs.** That is a wrong reading of #14 and it is the exact condition under
which forwarding cannot work. All corrected to per-address-**space** separation. One of
them (`multi_gpu::hash14_across_gpu`) had the conflation written into its own failure
message: *"must land in disjoint host VASes"* while asserting distinct **addresses**.

★ Address identity also removed a fact the suite was leaning on without saying so: the
mock minted host VAs out of `(proc, GPU)` bit lanes, and **six** assertions read
provenance off those bits (`l1_mean::host_va_lane`, `multi_gpu`'s `va >> 47 & 1`). The
host VA is the guest's number now and carries no provenance. `Published` gained
`memory: HostHandle`, whose `.isolate()` is `(Proc, GpuId)` **by type**.

### ★★★ Stage C — the decode — IS BLOCKED, and §2's open item is the blocker, confirmed

§2 left one thing open: *"where the destination-page content comes from if the core does
not emulate the copy."* **Measured on the tree, 2026-07-30: `kayfabe_mmu::walker::FbRead`
has ZERO implementors, and the emulated framebuffer is not modelled in this core at all.**
The C got the content free because QEMU performed the copy itself and could re-read the
destination page afterwards (`C: :6414`). We deliberately do not perform the copy — that
is the whole of stage B — so there is nothing to re-read.

So the decode splits cleanly, and only one half is buildable now:

- **Buildable today, behind the existing seam:** the GMMU-VER2 decoder as pure logic —
  `decode_page(fmt, fb: &dyn FbRead, page, {level, vabase}) → children + leaves`, driven by
  `GmmuFmt::decode_entry`, property-tested against synthetic FB images. It needs one Axis-B
  addition, `GmmuFmt::level_shift(level)`, because the VA bits a level strides are format,
  not core (the C's table is `{47,512},{38,512},{29,512},{21,256},{12,512},{16,32}` at
  `C: :8706-8708`). Reconstruct the VA from the page's **recorded level metadata**, never
  from a root walk — *"the guest fills a leaf page and links it under the root a separate
  push later"* (`C: :8681-8690`).
- **Not buildable until the FB shadow lands:** the production content source. A decoder
  wired to a seam nothing implements is not a stage, it is a stub with tests.

⇒ **Sequence the FB shadow before, or with, the decode.** Do not land the decoder alone.

Three things stage C must carry when it does land, all already established:

1. **Latch at write, decode at the guest's commit point** — the semaphore release. Decoding
   per write **livelocked on the bench** (`C: :8686-8690`); the latch half is built (it is
   `PtWrite`), the release trigger is not.
2. **The GA10x 512 MiB PD1-leaf case.** Detection is `(pde & 1) && level == 2`, aperture in
   bits `[2:1]` with **PTE** encoding (`0=VID, 2/3=SYS` — note this differs from PDE
   encoding, where vidmem is `1`), offset mask `va & 0x1FFF_FFFF` (`C: :4949`). The C
   handles it at **three** sites and they must stay consistent: `walk_pdb_root` **resolves**
   it (`:4949`), while `pt_enum` (`:8649`) and `cpt_decode_page` (`:8733`) **skip** it —
   never descend, never back. ★ Policy note: the skip is because the only known producer is
   the whole-FB identity alias, not because a 512 MiB leaf is invalid. The walker should
   **decode faithfully** and let the binding site apply the policy; a walker that silently
   drops a leaf size is the #13 round-4 gap.
3. **Apply strictly once, in submission order.**

★ **Acceptance criterion, and it is already written down as a marker in the code:**
`cb13_pt_write_capture_is_direct_no_root_reachability_needed` currently does **not** cover
the literal *"orphan leaf filled in push 1, linked under the root in push 2"* case, because
a leaf page only becomes tracked once a PDE pointing at it is decoded. Its doc comment says
so. Restoring that case is how the decode stage proves itself.

### Stage D — `#93` promote-ctx — unblocked in principle, two named costs

`gpu_promote_ctx.md` §4/§5 are still accurate and neither is dissolved by stages A/B:

- **§4** the codegen cannot express `promoteEntry[16]` (a fixed array of a nested struct).
  The decomposition in §4.1 stands: **generate the entry** (all scalars, full pinning
  stack), **hand-transcribe the 48-byte header** into `transcribed.rs`, index by stride.
  Knock-ons are three hardcoded counts in `oracle_layout.rs` and two negative assertions
  that flip.
- **§5** the consumer is a new seam: `route_control` returns the Case-2 ack **under the read
  lock, before any `Proc` is touched**, and binding needs `&mut Proc` plus a resolved
  `(GpuId, Pdb)` from a guest `hObject` that is not in scope there.

★★ Stage B **supplies the missing half of §5's step 2**: `Spine::pt_page_owner` is the
precedent for a device-global, projection-derived index that answers an ownership question
across procs, and `SharedDevice::with_proc_mut` is the one-lock-at-a-time applying pass a
Case-2 harvest needs. The lock-discipline objection is answered; the ABI cost is not.

★★ And key it on the **address space, not the RM client**: measured on hardware, two
concurrent processes **share one duplicated client**, so a client key aliases them — and
the C's own table is not dup-edge aware (§4, §2). Carries **seven** C defects; see
`c_bug_regression_matrix.md`. Notably an entry count clamped to **64** where the truth is
**16** (`NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES`, identical at 580 and 610), reading
560 bytes past the struct out of guest-writable memory. Do not port it.

### ★ Explicitly NOT built, and it is not this bug's cause

The **cross-channel fence** — blocking where a forwarded command acquires a semaphore a
page-table channel releases. It is real and recorded. It attaches at
`kayfabe_fwd::apply_pushbuffer`'s `SemRelease` arm, the same point the decode trigger
attaches. It is a separate, later concern.

---

## 11. ★★★ Stage C's content source — the verdict is (b): an ARCHITECTURE decision, NOT built

> **Question put to this pass:** is the missing content source (a) an implementation gap with a
> settled shape — *"the core already latches the phys-operand PT writes, so the payload is in
> hand; all that is missing is a store"* — or (b) a genuine architecture decision the owner must
> make?
>
> ★★★ **It is (b), and (a)'s premise is factually false.** Nothing was built. The evidence is
> below; every claim is cited and was read on the tree at `a1eca8a` / in the C artifact.

### 11.1 (a)'s premise is false: the payload is not in hand, and never was

- `kayfabe_arch::PushMethod::CeLaunchDma` carries **`{dst, len, dst_is_virtual}`** and nothing
  else (`rs: crates/kayfabe-arch/src/lib.rs:272-281`). There is **no source operand**, so the
  parser never sees the bytes a copy would move.
- `kayfabe_fwd::PtWrite` carries **`{gpu, page, aperture, owner, owner_pdb, bytes}`**
  (`rs: crates/kayfabe-fwd/src/lib.rs:2066-2080`) — the *identity* of a page that was written and
  *how much* of it, deliberately, because that is what a latch is. **No content, no `level`, no
  `vabase`.**
- `Vas::pt_pages` (`rs: kayfabe-core/src/gpu.rs:165`) is **write-only in production**: the only
  writers are `latch_pt_writes` and `SharedDevice::parse_pushbuffer`; the only readers are tests.

So "the payload of every write we intercept is in hand" describes a tree that does not exist.
Adding a source operand to Axis B would be cheap — but it would not help, for §11.2.

### 11.2 ★★★ A store of intercepted payloads cannot produce what the decode needs — three
independent reasons, any one of which is fatal

**(i) The orphan-leaf case is, by construction, written while the page is unclassified.**
`classify_ce` calls `Spine::pt_page_owner` (`gpu.rs:1159`), which consults **`pt_roots` only** —
and `Spine::refresh` seeds `pt_roots` **exclusively from declared PDB roots**
(`gpu.rs:2284-2292`: `pt_roots.insert((gpu, pdb.0 & !0xfff), (pid, pdb))`). ⇒ **today exactly one
page per VAS is classifiable as phys-operand.** Every deeper table page is `VaOperand` until a PDE
pointing at it is decoded. That is precisely the acceptance case §10 names — *"the guest fills a
leaf page and links it under the root a separate push later"* (`C: :8681-8690`) — so the leaf's
**fill** is classified as data and discarded, and by the time the link makes it interesting, the
write that filled it is gone. A payload store is empty exactly where the decode reads.

**(ii) The decode descends into pages that were never written on our watch.**
`nvkvm_m2_cpt_decode_page` recurses through `nvkvm_m2_pt_enum` (`C: :8737, :8656`), reading child
tables **by physical address, in FB *and* in guest sysmem** (`nvkvm_pt_rd64`, `C: :4891-4904`).
Those are reads of *memory*, not replays of *observed writes*.

**(iii) A page table has more than one writer, and the C hooks each separately.** The CE path is
`nvkvm_m2_ce_fb_write_hook` (`C: :6360` fills, `:6431` copies); the **CPU/BAR/PRAMIN** path is a
second, independent dirty arm inside `nvkvm_fb_write` itself (`C: :1398-1403`). Both converge on
one store. A capture that sees only the CE path sees only one producer.

★ **And the level metadata has the same shape of problem.** The `{page → pdb, vabase, level}`
triple stage C needs is populated in the C **only** by `nvkvm_m2_cpt_record`, whose three call
sites are all inside the **discovery root-walk** (`C: :8614, :8624, :8640`) — *"reset + rebuilt on
every recorded sweep"* (`C: :604`). It does not come from writes either.
**Good news, recorded so it is not re-derived:** the Rust does **not** need to port that sweep.
Level 0 is a declared fact (a PDB *is* its root page), so `decode_page(root) → children` yields
each child's `level+1` and `vabase` forward, from the root down — the metadata chain is
forward-populable in a way the C's never was. What it still requires is the **content of each page
in the chain**, which is §11.2 again.

### 11.3 The C's own words: the #13 fix depends on the emulator PERFORMING the write

`C: :4936-4952`, the PD1-leaf comment, states the causal chain outright: without the 512 MiB leaf
case *"chan_execute silently DROPPED every such PT write, the compute VAS's rebuilt subtree never
reached **our FB shadow**, its re-mapped buffers were never backed … and the host CE FAULT_PDE'd
one page past the last-backed leaf (#13, Xid 31)."*

The subtree reaches the shadow **because QEMU performs the copy into it** (`memcpy` /
`nvkvm_fb_host_ptr`, `C: :6413-6419`). Stage B removed the performing half. Nothing replaced the
store, and **nothing performs the intercepted copy at all today** — `PhysOperand` latches an index
and returns; `plan_doorbell` still rings the issuing channel on the host unconditionally. So the
question is not only *"where do we read the content back from"* but the one upstream of it:
★★ **who performs a phys-operand copy, and does its ring still reach hardware?**

### 11.4 What the production `FbRead` actually is in the C — and why it is not a shadow

`nvkvm_fb_read` / `nvkvm_fb_write` are the emulated device's **memory**, and they are a **hybrid**:
a sparse `g_malloc0(4096)` page store (`C: :906-919`) *overlaid by real host GPU memory* wherever
a range is host-backed (`nvkvm_fb_host_overlay` / `nvkvm_fb_host_ptr`, `C: :1445-1457, :5528`).
Where the overlay hits, **the host GPU co-writes the bytes behind us** — the same uninstrumented-
channel class `c_rust_trace_differential.md` names for `pci_dma_map`.

And it is **device-wide, not a stage-C detail.** The same two functions serve BAR0-PRAMIN
(`:1478`), BAR1/BAR2 (`:4623`, `:4690`), the BAR1/BAR2 page directories (`:3522`, `:3529`), channel
USERD `GP_PUT`/`GP_GET` (`:4132-4133`), the instance block's PDB words (`:4795-4796`), GPFIFO entry
fetch (`:6086`) and semaphore payloads (`:4333-4335`).

⇒ Implementing `FbRead` "for the decode" **is building the emulated device's memory plane**, and
its home is an **explicitly open question in this repo's own docs**: the planned `nvkvm-regs` crate
(*"BAR0 map, intr tree, MSI-X routing, PRAMIN window, read-native overlay policy"*,
`mode2_rust_rewrite_architecture.md` §4.2) **was never built**, and `mode2_gsp_port_plan.md`
records **`[open] O1 — where does the rest of the register model live?`** … *"PTIMER, the display
fuse, the PCI-config mirror, PRAMIN … This plan does not decide it."*

That is the (b) trigger as the owner stated it, met twice over: a new authoritative store of
device memory, in a core whose premise is that it holds none, whose owning crate is undecided.

### 11.5 ★★ A second, adjacent departure found while verifying — stage B folded TWO C predicates
into one

The C has **two different decisions**, and they are not the same predicate:

| decision | C predicate | site |
|---|---|---|
| **execute** — host CE vs CPU emulation | `m2cexec && !mscrub && !remap && !src_phys && !dst_phys && is_user_ce(chan_client)` | `C: :6310` |
| **capture** — is this a PT write? | the fb-write hook fires on the **resolved physical** of every emulated FB write | `C: :6360, :6431` |

So in the C, **every kernel-CE copy is CPU-emulated** — including the FB-alias PT write, which is
*virtual*-dst and would pass a purely operand-carried test — and so are scrubs and fills. Stage B's
split (`379f712`) is right about **capture** and silently answers **execute** as well, by routing
everything non-phys to "forward it, let hardware execute it"
(`rs: kayfabe-fwd/src/lib.rs:2281-2286`). Whether kernel/CeUtils/scrubber work is forwarded or
emulated is a **separate decision the Rust has not made**, and it is upstream of the content
question: if the answer is "emulated", the content is free exactly as it was in the C.

### 11.6 The options, and the recommendation

**Option 1 — device memory as an effect port outside the pure crates (port the C's shape).**
Keep `walker::FbRead` as the abstract seam; implement it in the shell over a sparse owned page
store plus the host-vidmem overlay where a range is host-backed. Core purity is preserved (this is
structurally `Vmm`). *Costs:* a software CE must exist to perform intercepted copies — which the
execution-plane doctrine says never to build, and which the C nonetheless does for kernel channels;
the port is a **second** memory port; and where the overlay hits, it is an *alias* of memory
hardware also writes, not a shadow we own.

**Option 2 — PT pages RAM-backed in guest-physical space, read back through `Vmm`.**
This is the posture §4.4's **audit C1** already argues for: *"PT pages are RAM-backed and the guest
writes them natively; we capture via the CE-write hook and decode at the commit point"*, and the
mechanism exists in the port already (`Vmm::map_read_native`, cap 7). Core adds an **FB-address →
backing index**, not a content store; content read-back is `gpa_read`; one memory port, no aliasing
of host vidmem. *Costs / unknowns:* (i) it **drops a capture channel the C had** — a guest **CPU**
write to a PT page becomes invisible, where `nvkvm_fb_write`'s dirty arm saw it (§11.2 iii); (ii) it
still needs an answer to *who performs the intercepted copy* (§11.3); (iii) it needs the BAR/PRAMIN
aperture model, i.e. **O1**.

**Option 3 — the literal (a): a core-owned store of intercepted payloads.** ⊘ **Rejected on the
evidence**, not on taste: it cannot serve the orphan-leaf case (§11.2 i), cannot serve descent
(ii), and sees one of at least two writers (iii). Generalising it until it works means shadowing
**every** CE destination — which is Option 1 with the store in the wrong crate.

★ **Recommendation, for the owner to rule on.** Take **Option 2 as the target posture and Option 1's
port shape as the seam** — the core keeps naming pages by FB address through `FbRead` and never
learns a GPA for a page table, so whichever backing **O1** settles on is a shell decision that does
not reach a logic crate. **But decide §11.3 first**: *who performs a phys-operand copy, and does its
channel's ring still reach hardware?* Every version of the content question is downstream of that
answer, and if it is "we perform it", the content is free — which is exactly how the C got it.

### 11.7 What stage C may and may not do until this is ruled on

- **May not** land the FB shadow, the decoder wired to a production source, or any store of device
  memory content. §10's own instruction stands: *"do not land the decoder alone."*
- **May** land, if the owner wants motion meanwhile: `GmmuFmt::level_shift(level)` (Axis-B; the C's
  table is `{47,512},{38,512},{29,512},{21,256},{12,512},{16,32}`, `C: :8706-8708`) and the pure
  `decode_page` over a synthetic `FbRead`, property-tested — **as a decoder with tests and no
  production caller**, which is what §10 already calls "a stub with tests".
- ★ **Unchanged and still binding:** the 512 MiB PD1 leaf must **decode faithfully** at the walker
  and be dropped by *policy* at the binding site. Re-verified at all three C sites this pass:
  `walk_pdb_root` **resolves** it (`C: :4949`, offset mask `va & 0x1FFF_FFFF`, PTE aperture
  encoding `0=VID, 2/3=SYS`), `pt_enum` **skips** it (`C: :8649`) and `cpt_decode_page` **skips** it
  (`C: :8733`) — and both skips carry the same reason, *"its only known producer is the CeUtils
  whole-FB identity alias"*, i.e. **policy about that alias, not a property of a 512 MiB leaf.**

## 12. ★★★ §11.3 ANSWERED BY THE OWNER (2026-07-30) — stage C is UNBLOCKED

The blocking question was *who performs a phys-operand copy?* The ruling is not a boolean over
whole copies. It is a **decomposition by representability**:

1. **We perform a copy only where it is UNREPRESENTABLE by real NVIDIA CE** — i.e. where an
   operand is *fabricated*: our fake PDB / GPGA / emulated-FB space, which no real engine can be
   pointed at because the addresses do not denote real device memory.
2. **Everything representable goes to real hardware.** If the operands can be expressed as GPU VAs
   and source and/or destination is VRAM, issue a **real CE copy** — that is normally *faster*
   than a CPU `memcpy`, not merely more faithful.
3. **A single privileged CE request may SPLIT.** If a privileged copy also covers non-fake memory
   that a real CE *can* express, those sub-copies are issued to **real CE**; only the
   unrepresentable remainder is ours.
4. **The executor is the ISOLATE in both cases** — real CE submission and our own copy over
   VRAM-backed mappings alike. Never the hypervisor process, never the pure core.

### 12.1 Why this dissolves §11.2's three objections

**(i) The orphan leaf.** §11.2 objected that a fresh leaf is unclassified at fill time, because
`pt_page_owner` consults `pt_roots` and exactly one page per VAS is classifiable. That objection
assumed the capture criterion is *"is this a page table?"*. **Under this ruling the criterion is
"is the destination representable?"** — a property of the *address*, not of our knowledge about
its role. A fresh leaf living in fabricated space is therefore performed-by-us, and its content
held, **before** any PDE points at it. The classification can arrive later; the bytes are already
ours. ★ This is the reason the ruling is stronger than either option as written.

**(ii) Descent by physical address.** The decoder descends into child tables read by physical
address. Under the split, every page in fabricated space was written by *us* and is readable from
the isolate's VRAM-backed mapping of that aperture; every page outside it is real memory the host
can read directly. Both arms have a source.

**(iii) At least two writers.** The CE path is covered by construction. The second writer —
CPU/BAR/PRAMIN stores into fabricated space (`C: :1398-1403`) — is covered by the *same*
principle rather than a second mechanism: **a write into fabricated space is ours**, because
there is no real engine it could have gone to. It still has to be *wired*, and it is not wired
today.

### 12.2 Where this puts the content store — and why the core stays pure

**Option 3 (a core-owned store of intercepted payloads) remains rejected**, and this ruling does
not revive it. The content lives where the memory lives: in the **isolate's VRAM-backed mapping
of the fabricated aperture**. The core holds the address table and decides *what*; the isolate
holds bytes and does *it*.

⇒ `walker::FbRead` keeps its abstract seam (Option 1's shape) and gets its production
implementation over Option 2's backing. **This is exactly §11.6's recommendation, with the piece
it was missing.** The core never learns a GPA for a page table and never owns device memory, so
whichever backing **O1** eventually settles on stays a shell decision.

★★ **The bound that makes this safe to build:** we shadow the **fabricated aperture only** — not
"every CE destination", which is what made Option 3 collapse into Option 1 with the store in the
wrong crate. The fabricated aperture is memory we invented and therefore already own.

### 12.3 What stage C may now do, and what is still open

**May build:** `GmmuFmt::level_shift`, `decode_page`, and the walker wired to a **real** `FbRead`
implemented over the isolate's fabricated-aperture mapping — no longer a decoder with tests and
no production caller.

**Still open, and must not be guessed:**
- ⚠ **One ambiguity in the ruling to confirm before relying on it.** "Execute in host userspace…
  if source and/or dest is VRAM" and "our memcpy for VRAM-backed addresses, executed by the
  isolate" can be read two ways. This document takes: *representable ⇒ real CE; unrepresentable ⇒
  our copy, performed in the isolate against the VRAM-backed mapping.* If the intent was instead
  that VRAM involvement alone selects our copy, §12.1(i) still holds but the split boundary moves.
- **The CPU/BAR/PRAMIN writer (ii) is not wired.** Named here so it is not mistaken for done.
- **Splitting a request into representable and unrepresentable sub-copies needs a range
  algebra** — the operand ranges must be partitioned, not classified whole. That is new code with
  an obvious mean test (a copy straddling the boundary must produce byte-identical results to the
  same copy issued wholly by either path where both are legal).
- ★ **§11.5's finding is untouched and still owed:** stage B folded the C's *execute* predicate
  (`C: :6310`, which CPU-emulates every **kernel**-CE copy) into its *capture* predicate. This
  ruling is about execute. The two must now be re-separated deliberately rather than by accident.

## 13. ★★★ STAGE C1 + C2 — BUILT (2026-07-30). What landed, and the one finding that
contradicts a stated premise

### 13.1 C1 — the two predicates, separated (§11.5 discharged)

`kayfabe_fwd::ce_executor_c(work, origin, src_is_virtual, dst_is_virtual) -> CeExecutor`
is the C's execute predicate (`C: :6310`), ported literally; `classify_ce` remains the
capture one. `PushMethod::CeLaunchDma` grew `src` / `src_is_virtual` / `work`, because
three of the C's five live conjuncts read the source operand and the work kind and the
decode carried neither — **a decision you cannot state is a decision you answer with
whatever you already have**, which is precisely what stage B did.

One conjunct is deliberately not modelled: `m2cexec`, the C's bench debug switch. This
port has no mode in which execution forwarding is off.

`ChannelOrigin::of(ProcId)` ports `is_user_ce(chan_client)` onto the **proc** rather than
onto a runtime-accumulated client list — `Gpu::system` already *is* the guest-kernel
component (§12.27). A strengthening: the C's list was populated by observation, so a
client it had not yet seen read as *not* user-CE.

### 13.2 C2 — the representability split

- `Representability { HostBacked, Fabricated, PhysicalOperand, Untracked }` — a property
  of the **address**. `HostBacked` = host-published in the owning `Vas` at the identical
  host VA (stage A's identity law is what makes the guest's own number usable by real
  hardware). `Fabricated` = declared, nothing host-side. `PhysicalOperand` =
  unrepresentable *by construction*, no lookup. `Untracked` = forwarded, never guessed;
  its safety net is the #14 ring gate, not this classification.
- `AddressTable::spans(va, len)` — the range query beside the point query. A wrapping
  `va + len` is **clipped at the top of the address space, never wrapped**: honouring the
  wrap would let a hostile length aimed at the top reach a mapping at the bottom.
- `partition_ce(...) -> Vec<CeSpan>` — the algebra. **Both operands are partitioned and
  the partitions are intersected**, because a sub-copy is hardware's only if BOTH ends are
  expressible (`!src_phys && !dst_phys` in the C's one conjunction). Bounded by
  `MAX_CE_SPANS` per request and `MAX_CE_SPANS_PER_PARSE` per parse, both **loud**
  refusals — a truncated partition is a silently dropped tail, i.e. `#13 CE-DROP` rebuilt.
- `VerbPlan::CeSplit { vas, subs }` + `RmBackend::ce_copy` — §12.4's *"the executor is the
  isolate in both cases"*, made structural: the core builds a plan and has no way to move
  a byte. ONE verb, with the engine as a field of the instruction, because representability
  is address-plane knowledge the backend does not hold and must not appear to.

**Three measured departures from the C's predicate** (`ce_representability_split.rs` pins
each as a value):

1. a **guest-kernel** copy between host-backed ranges — C: ours; §12: real hardware;
2. a **user** scrub/fill of a host-backed range — C: ours; §12: real hardware;
3. ★ a **user** copy into *fabricated* space — the C's predicate alone would hand it to
   the host engine, which would resolve nothing; the C survives only because a separate
   map-on-touch step (`C: :6267-6295`) backs the destination first. §12 keeps it, with no
   second mechanism.

### 13.3 Fabricated VRAM in a guest *userspace* GPU VA — no special case exists

The owner's rare corner (give it a real host backing so a real engine can be pointed at
it) is implemented as **publication**, i.e. the port's ordinary path, and the classifier
then answers `HostBacked` on its own. **The dummy backing IS the representation.** There
is no code anywhere that knows this case exists.

The uninspected userspace fast path stays uninspected, and that is structural: the split
runs inside the pushbuffer parser, and the parser only runs where the core is already the
mediator.

### 13.4 ⚠ FINDING — there is NO read-at-invalidate, in this port or in the measured C

The premise that would make an uninspected userspace copy engine safe *by mechanism* —
"the table is forward-populated by RPC **and PDB-read-at-invalidate**, so a fabricated
page table scribbled through an uninspected engine is recovered at the next invalidate" —
**does not hold**:

- **This port:** `PushbufferOutcome::invalidates` has **no production consumer**. The
  parser records `(pdb, membar)`; nothing re-reads a page directory. Pinned by
  `there_is_no_read_at_invalidate_and_the_table_is_unchanged_across_one`.
- **The C artifact:** `mode2_address_table.md` §5's own ★ CORRECTION (2026-07-22, audit
  S3, #14 round-6) records **both** invalidate transports measured at **zero** occurrences
  on the Mode-2 GSP-emulated compute path, and concludes the two co-equal populate sources
  are bind-time RPC bindings and the **observed CE PT-write** — *"§4.2's
  'read-at-invalidate is load-bearing' … [is] false for the GSP-emulated compute path"*.

⇒ Correctness rests on **witnessing the CE page-table write**. Leaving the userspace path
uninspected is nonetheless correct on the measured path — the page-table writer is the
guest *kernel*'s copy-engine utility, on a channel the core does mediate — but for that
reason, not because of an invalidate contract. **If a guest userspace channel ever becomes
the writer, nothing currently recovers.**

### 13.5 Still not built, deliberately

`FbRead`'s production implementation and the decoder (stage C3). The `Ours` arm's real
backend is `NOT_ON_THIS_RUNG` in `HostRmBackend` — it needs the isolate's mapping of the
fabricated aperture, which is exactly what C3 builds. The `HostCe` arm is refused there
for the same reason `ring_doorbell` is. Returning `Ok(())` for a copy that moved no byte
is the forged-completion failure `mode2_real_forward_not_fake` forbids.

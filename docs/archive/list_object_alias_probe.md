> ⊘⊘⊘ **ARCHIVED — THIS DESCRIBES A SUPERSEDED ARCHITECTURE. IT IS REFERENCE, NOT CURRENT.**
> The live design is `docs/design/THE_DESIGN.md`. This file is kept because its *measurements*,
> its *ogkm findings* and its *reasoning* remain useful — its **architecture does not**. Do not
> implement from it, and do not cite it as current. Archived 2026-09-21 (w822).

# `NV01_MEMORY_LIST_OBJECT` — does a slice ALIAS its parent's pages, or COPY them?

**STATUS: ANSWERED — ★★★ ALIAS (w747, 2026-09-15).** Measured on a real **GA106 / RTX 3060**,
**NVIDIA UNIX Open Kernel Module 580.159.04**, bare metal (no KVM, no guest), `REV_UNDER_TEST =
d0347a5869744de86a87f9348bc94f45ef2d5e47`. Three consecutive runs, identical.
`traces/w747_list_object_alias/`.

    ★★★★★ W747 STEP 5 VERDICT = ALIAS — wrote B 0x747b0002 through the PARENT's page 2;
                                 the SLICE now reads 0x747b0002 (it read 0x747a0001 before)
    ★★★★★ W747 step 8 phys    = parent page 2 at 0x0000000003112000,
                                 slice at 0x0000000003112000 — THE SAME PHYSICAL ADDRESS
    W747_VERDICT=ALIAS  W747_REVERSE=ALIAS  W747_CONTROL=HELD  W747_KNOWN_POSITIVES=HELD

⇒ **A `LIST_OBJECT` slice is the parent's pages, not a copy of them.** Leg B's premise holds.

⚠ **Two results that bound it, and both change what leg B has to do — read them before
building on the verdict.** They are §§ *"The gate is CAP_SYS_ADMIN"* and *"The lifetime window
is real and silent"* below.

The predictions are not edited, only marked HELD / REFUTED.

## Why this is the gate

Leg B of the USERD design rests on one property. If `NV01_MEMORY_LIST_OBJECT` (class `0x83`,
`cl84a0.h`: *"List of page numbers relative to the start of the specified object"*) **aliases**
the parent's physical pages, then the scratchpad can mint a handle naming **one 4 KiB page** of
the single reserved store, the isolate names that as `hUserdMemory[0]`, and **the channel is born
on the isolate** — which preserves its privilege and `ProcessID` (constraints 26 and 30).

If it **copies**, the guest rings a USERD that hardware never reads, and — this is the part that
makes the question worth a box — **every counter we have would show green**. USERD is referenced
by *physical address* (`memdescGetPhysAddr(pUserdSubDeviceMemDesc, AT_GPU, 0)`,
`kernel_channel_gm107.c:328`, 4 KiB-attributed at `:689`) and never through PT\*/PD\*, so nothing
in our mapping plane would ever notice the divergence.

## The discriminator is a write AFTER creation — and only that

⊘ Reading the parent's value through the slice **immediately after creating it** proves nothing:
that result is identical for an alias and for a copy-at-creation. This campaign's most expensive
recurring defect is exactly that shape (*"correct by accident under a temporary condition"*).

The rung's order, and the only order that discriminates:

| step | action | what it settles |
|---|---|---|
| 1 | allocate a 4-page FBMEM parent, CPU view over it | premise |
| 2 | write **A** into page *N*; write sentinel **S** into page *M* (`M != N`) | seeding |
| 3 | create slice `L_N` over page *N* only, and control slice `L_M` over page *M* only | the objects |
| 4 | read `L_N` → expect **A**; read `L_M` → expect **S** | necessary, **not sufficient** |
| 5 | ★★★ write **B** through the PARENT's page *N*; read `L_N` | **B ⇒ ALIAS · A ⇒ COPY** |
| 6 | write **C** through `L_N`; read the parent's page *N* | both directions must agree |
| 7 | read `L_M` again | ⚠ must still be **S** — not zero, not B |
| 8 | `GET_SURFACE_PHYS_ATTR` on parent@`N*4096` and on `L_N`@0 | direct equality, not inference |

★ Step 7 is the control and it must be **positively identified**, never merely "not B". w744's
own rung shipped a control whose *"untouched"* VA had been mapped by the probe itself; a control
that reads zeros because its view is dead is indistinguishable from one that is genuinely
isolated. Seeding **S** first and requiring **S** back is what makes the difference observable.

## Pre-registered predictions — each with the value that refutes it

| # | prediction | refuted by | result |
|---|---|---|---|
| P1 | the `0x83` alloc is **accepted** (`status=0x0000`) as **root** on a GSP-client GA10x | any non-zero status | ⊘ **REFUTED AS WRITTEN, HELD AS CORRECTED.** Root is not enough — see *"the gate is CAP_SYS_ADMIN"*. With the capability: accepted, `handle 0xcafe0005` |
| P2 | ★★★ **ALIAS** — step 5 reads **B** through `L_N` | step 5 reads **A** (or anything else) | ★★★ **HELD.** `0x747b0002` |
| P3 | the reverse direction also aliases — step 6 reads **C** through the parent | the parent still reads **B** | **HELD.** `0x747c0003` |
| P4 | the control `L_M` reads **S** at step 4 **and** at step 7 | reads **B**, or reads `0`, at either point | **HELD.** `0x74750004` both times; and the control's own page, written `0x747d0005` through the parent, reached it ⇒ it is a live view, not an inert one |
| P5 | `phys(L_N@0) == phys(parent@N*4096)`, aperture `VIDMEM` both | any inequality, or a refusal | ★★★ **HELD.** Both `0x0000000003112000`, aperture 0 VIDMEM. ⊘ `memFormat` differs (parent `0x6`, slice `0x0`) and `contigSegmentSize` differs (`0xe000` vs `0x1000`) — the slice is one page of a longer contiguous run and carries the `format` we passed, which is `0` |
| P6 | a slice naming a **foreign** client's parent via `hClient`/`hParent` is **accepted** | any non-zero status | **HELD.** `status 0x0000 NV_OK` — ⚠ but the *minting* client also held `CAP_SYS_ADMIN`; see the scoping below |
| P7 | the slice is **dupable** into a second client (`memlistCanCopy_IMPL` returns `NV_TRUE`) | any non-zero status | ⊘ **NOT MEASURED BY CHOICE** — it needs a second `NV_ESC_RM_DUP_OBJECT` escape, which constraint 26's countability gate forbids. Still a reading, not a measurement |
| P8 | ⚠ freeing the **parent** with a live slice outstanding is **accepted**, and the slice then **silently serves stale physical memory** — no refusal, no fault | RM refuses the free, or the read faults/refuses | ⊘⊘⊘ **HELD, AND WORSE THAN PREDICTED** — see below |
| P9 | as a **non-root** uid the same alloc is refused **`0x1b NV_ERR_INSUFFICIENT_PERMISSIONS`** | any other status, accepted included | **HELD**, on the same box, same driver, same binary, `uid 65534` |

> ### ⊘⊘ CORRECTED BEFORE THE RUN (same day) — P9's REFUTER WAS THE WRONG NUMBER
> This row first named **`0x1f`** as `NV_ERR_INSUFFICIENT_PERMISSIONS`. It is **`0x1b`**
> (`nvstatuscodes.h:56`); `0x1f` is **`NV_ERR_INVALID_ARGUMENT`** (`:60`) — the code a
> malformed parameter block gets, which is the *most likely* refusal this rung will actually
> see. ⇒ the original refuter would have read a bug in my encoder as RM enforcing a privilege
> rule, and the finding would have been a fact about the probe. ⚠ Corrected in the table
> above and in `listobj.rs`'s decode table, which carried the same wrong row and now asserts
> both codes in both directions. Same class the repo already names: **a citation that is not
> read is a guess with a reference attached.**

### Where P1, P8 and P9 come from — read before the run, not after it

- **P9 / P1.** `resource_list.h:631-640` gives `NV01_MEMORY_LIST_OBJECT` the flag
  **`RS_FLAGS_ALLOC_PRIVILEGED`**, and `alloc_free.c:649-658` refuses
  `NV_ERR_INSUFFICIENT_PERMISSIONS` (**`0x1b`**) when `privLevel < RS_PRIV_LEVEL_USER_ROOT`. ⇒ **the class is
  root-only.** That is a *design* fact, not a probe detail: it says which of our processes may
  mint a slice at all, and it is the first thing this rung must confirm rather than assume.
- **P1's other gate.** `memlistConstruct_IMPL:88-101` returns `NV_ERR_NOT_SUPPORTED` outright
  unless `hypervisorIsVgxHyper()`, or `IS_VIRTUAL && privLevel >= KERNEL`, or
  **`IS_GSP_CLIENT(pGpu)`**. Bare metal is none of the first two, so the run stands entirely on
  GSP being in use. ⊘ A box whose driver has GSP disabled would refuse `0x56` and that would be
  an **environment** result, not RM's ruling on aliasing — it must be reported as such.
- **P2 / P5.** `memlistConstruct_IMPL`'s FBMEM arm takes
  `baseOffset = memdescGetPhysAddr(src_pMemDesc, AT_GPU, 0)` and then, per page,
  `memdescSetPte(pMemDesc, AT_GPU, i, newBase + baseOffset)`. **No allocation and no copy
  appears anywhere in the function**, which is what the class comment says in as many words:
  *"No memory is allocated: only a memory descriptor and memory object are created."*
- **P8.** Nothing in that function takes a reference on `src_pMemDesc`; only the Windows-VM
  `hHwResHandle` path bumps a refcount, and that is on the *hardware resource*, not the memory.
  ⇒ the slice is a **snapshot of physical addresses that subscribes to nothing** — which is
  exactly what constraint 31 already states and this rung is the measurement of.

⚠ **A source reading is a prediction, not a result.** Every row above is decoded against
`research_clones/ogkm-580.159.04/src/common/sdk/nvidia/inc/nvstatuscodes.h` when it comes back,
and the run does **not stop at the first refusal** — w744 lost a lane to two refusals
(`0x19 INSERT_DUPLICATE_NAME`, `0x26 INVALID_DEVICE`) that were its own setup rather than RM's
ruling.

## ★★★★★ THE GATE IS CAP_SYS_ADMIN, NOT uid 0 — measured twice, two ways of losing it

    ⊘  W747 slice L_N = page 2 attr PAGE_SIZE_4KB      REFUSED status 0x001b NV_ERR_INSUFFICIENT_PERMISSIONS
    ⊘  W747 slice L_N = page 2 attr PAGE_SIZE_DEFAULT  REFUSED status 0x001b NV_ERR_INSUFFICIENT_PERMISSIONS
    ==  W747 list-object-alias = gpu 0, euid 0, …

**`euid 0` and the class still refused.** The gate is `RS_FLAGS_ALLOC_PRIVILEGED` →
`privLevel < RS_PRIV_LEVEL_USER_ROOT`, and RM's `privLevel` comes from
`os_is_administrator()`, which on Linux is **`capable(CAP_SYS_ADMIN)`**
(`ogkm-580: kernel-open/common/inc/nv-linux.h:537`, `NV_IS_SUSER`). The box's container had

    CapEff: 00000000a80405fb   (cap_sys_admin absent; cap_chown … cap_setfcap present)

⇒ ★★★ **"root" is not RM-admin. The discriminator is CAP_SYS_ADMIN, not uid 0.** A VMM
running as uid 0 inside any container, namespace or service manager that drops
`CAP_SYS_ADMIN` **cannot mint a `LIST_OBJECT` at all**, and the refusal arrives as a
permissions status with nothing in it naming a capability. ⚠ This is a *deployment*
constraint on leg B, not a probe detail, and it was measured rather than predicted — the
pre-registration said "root-only" and root was not enough.

(Run 1, a CUDA container on a GA106 at driver **580.173.02** —
`traces/w747_list_object_alias/ga106_580.173.02_container_noCAP_SYS_ADMIN_REFUSED.log`.)

★ Two rows that DID hold in that run, and they matter because they attribute the refusal:
`nvidia-smi -q` reported **`GSP Firmware Version: 580.173.02`**, so `memlistConstruct`'s
`IS_GSP_CLIENT` gate is satisfied on this class of box — the refusal is the privilege gate
and **only** the privilege gate. And the seed/readback control passed
(`page 2 = 0x747a0001, page 0 = 0x74750004, both read back through the parent`), so the
parent, its CPU view and the pattern plumbing are all live.

### ★ And the matched positive/negative pair, on ONE box

`uid 0` with `CapEff: 000001ffffffffff` ⇒ **accepted**. The same binary on the same box under
`setpriv --reuid=65534` ⇒ **`0x001b NV_ERR_INSUFFICIENT_PERMISSIONS`**, while `alloc_vidmem`,
its CPU view and the seed/readback in that same unprivileged run all **succeeded** — so the
refusal is scoped to class `0x83` and to the *allocating* client's privilege, not to the GPU,
the node permissions (`crw-rw-rw-`) or the memory plane.

⇒ ★★★★★ **THE CONSEQUENCE FOR LEG B, and it is the opposite of what the cross-client row
first suggests.** The P6 row shows a *second client* can mint a slice over the *first
client's* object — which reads like *"the isolate can mint its own USERD handle, no hand-over
needed, constraint 30 never arises"*. ⊘ **It does not follow.** In that row client B lived in
the same process and therefore held the same `CAP_SYS_ADMIN`. A real per-proc isolate is
unprivileged **by design**, and the gate is on the allocating client, so it would be refused
`0x1b` exactly like the `uid 65534` arm. ⇒ **the scratchpad must mint the slice and hand it
over**, which puts constraint 30's question — *does the shared artefact carry the creator's
privilege or process identity?* — squarely back on the critical path, and makes P7's dup the
next thing that has to be measured under its own reviewed escape.
⚠ Stated as a **derivation from two measurements of the same gate**, not as a third
measurement: nothing here ran a client whose process lacked the capability while naming a
foreign parent.

## ⊘⊘⊘ THE LIFETIME WINDOW IS REAL, AND IT IS SILENT — constraint 31, measured

    ★★★ W747 lifetime free  = the PARENT was freed with a live slice outstanding — RM did NOT refuse
    info  W747 lifetime read  = through the slice: 0x747b0002 before the free, 0x00000000 after
    ⊘⊘⊘ W747 lifetime stale = THE SLICE SERVES THE NEW OWNER'S BYTES. A fresh allocation wrote
        0x747d0005 at page 2 and the slice — minted over a parent that no longer exists — reads
        0x747d0005.

Three facts in three lines, and the third is the one constraint 31 exists for:

1. **RM does not refuse the free.** The slice holds no reference on its parent — which is what
   `memlistConstruct_IMPL` reading `memdescGetPhysAddr` and never taking a ref predicted.
2. **The page was scrubbed**, not left stale: the slice read `0x00000000` immediately after the
   free. ⊘ ⚠ **Do not read that as containment.** It is RM's vidmem scrubber running on free,
   and it says nothing about who owns the page next.
3. ★★★ **A fresh allocation landed on the same physical page and the orphaned slice served its
   bytes.** No refusal, no fault, no counter anywhere. That is constraint 31's sentence —
   *"a page now owned by guest process B is still reachable through machinery minted for guest
   process A"* — reproduced end to end in three lines of output, on hardware, and the only
   reason it reads as benign is that nothing in the system is in a position to complain.

⇒ **Constraint 27's barrier must extend from mappings to HANDLES, exactly as constraint 31
says.** ⊘ And revalidation cannot substitute: the slice's page list is taken at creation and
subscribes to nothing, so there is no moment at which a checker could notice.

## Known-positives — what makes a green result mean anything

1. **The control arm (P4).** A `LIST_OBJECT` over a *different* page must **not** see the
   pattern, and must be positively identified by its own sentinel. Without it a rung that always
   prints ALIAS is indistinguishable from one that works.
2. ★★★ **The grader must be able to say COPY.** The rung feeds the *same* verdict function a
   deliberately **stale snapshot** — page *N*'s bytes captured into host memory at slice-creation
   time — and asserts it grades that as **COPY**. A test that can only print ALIAS is not a test.
3. **A second independent object.** The verdict function is also run against a wholly separate
   vidmem allocation, which must grade **COPY/independent** for the same reason.

## What this does NOT settle

⊘ Scoped to **RM's memory-descriptor semantics**. It says nothing about whether the FIFO accepts
such a handle as `hUserdMemory[0]`, nor about what hardware does with the resulting instance
block — those are separate rungs with separate failure modes. And it is **one chip, one driver
build**; `a_capture_derived_table_expires_as_a_vendor_regression` applies.

**No constraint is relaxed by this rung.** It allocates, reads, writes and frees inside one
process's own RM client(s); constraints 26/30/31 are the things it is measuring *for*, not
things it stands on.

# `NV01_MEMORY_LIST_OBJECT` — does a slice ALIAS its parent's pages, or COPY them?

**STATUS: PRE-REGISTERED (w747, 2026-09-15).** Predictions below are committed **before** a GPU
box exists. The measurement is `rmladder --list-object-alias` on **bare metal** (real GPU, no
KVM, no guest). This block is updated in place with the measured result; the predictions are
never edited, only marked HELD / REFUTED.

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

| # | prediction | refuted by |
|---|---|---|
| P1 | the `0x83` alloc is **accepted** (`status=0x0000`) as **root** on a GSP-client GA10x | any non-zero status |
| P2 | ★★★ **ALIAS** — step 5 reads **B** through `L_N` | step 5 reads **A** (or anything else) |
| P3 | the reverse direction also aliases — step 6 reads **C** through the parent | the parent still reads **B** |
| P4 | the control `L_M` reads **S** at step 4 **and** at step 7 | reads **B**, or reads `0`, at either point |
| P5 | `phys(L_N@0) == phys(parent@N*4096)`, aperture `VIDMEM` both | any inequality, or a refusal |
| P6 | a slice naming a **foreign** client's parent via `hClient`/`hParent` is **accepted** | any non-zero status |
| P7 | the slice is **dupable** into a second client (`memlistCanCopy_IMPL` returns `NV_TRUE`) | any non-zero status |
| P8 | ⚠ freeing the **parent** with a live slice outstanding is **accepted**, and the slice then **silently serves stale physical memory** — no refusal, no fault | RM refuses the free, or the read faults/refuses |
| P9 | as a **non-root** uid the same alloc is refused **`0x1f NV_ERR_INSUFFICIENT_PERMISSIONS`** | any other status, accepted included |

### Where P1, P8 and P9 come from — read before the run, not after it

- **P9 / P1.** `resource_list.h:631-640` gives `NV01_MEMORY_LIST_OBJECT` the flag
  **`RS_FLAGS_ALLOC_PRIVILEGED`**, and `alloc_free.c:649-658` refuses
  `NV_ERR_INSUFFICIENT_PERMISSIONS` when `privLevel < RS_PRIV_LEVEL_USER_ROOT`. ⇒ **the class is
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

# Mode-2: is there a SECOND crossing for the GR context buffers in the emulated FB?

**Answer: YES.** It exists, it was settled architecturally on **2026-06-04**, and it was **built and
ran** — in two generations. The null result is not available.

**Status:** answered 2026-08-10. Companion to `mode2_dataplane_decision.md` §5 (the settled ruling),
`mode2_userbuf_vidmem_passthrough.md` (the machinery), `mode2_guest_ram_crossing.md` (the *first*
crossing — sysmem) and `mode2_gpga_memfd.md` (where fabricated GPGA bytes should live).

Trees: `C:` = this repo, `rs:` = `/workspace/nvkvm-rs`,
`ogkm-580:` = `research_clones/ogkm-580.159.04/`.

> ### ✔ Verified independently before this page was committed
> Three load-bearing claims were re-checked by hand rather than taken from the analysis:
> 1. **`0x51` is `NV_ERR_NO_MEMORY`** — `ogkm-580: kernel-open/common/inc/nvstatuscodes.h:110`,
>    `NV_STATUS_CODE(NV_ERR_NO_MEMORY, 0x00000051, "Out of memory")`. ✔ exact.
>    (`0x1B` = `NV_ERR_INSUFFICIENT_PERMISSIONS`, `:56`. ✔)
> 2. **`m2exec` defaults ON while its call-site comment says OFF** —
>    `C:src/qemu/nvkvm_gpu_emul.c:9929` = `DEFINE_PROP_BOOL("m2exec", …, true), /* …(always on) */`
>    against `:3884` = `/* M5.7 EXECUTION PLANE (gated m2exec, default off): …`. ✔ both, verbatim.
> 3. **PATCH is not privileged** — `ogkm-580: …/gr/kernel_graphics_context.c:1127` (MAIN) =
>    `MEMDESC_FLAGS_GPU_PRIVILEGED`; `:1206` (PATCH) = `MEMDESC_FLAGS_OWNED_BY_CURRENT_DEVICE`. ✔

---

## §1 REFUTED — lead with these

### ⊘⊘ R1 — the commissioning hypothesis, both halves, each independently

> *"If the host's context is genuinely self-mapped, the host GR engine needs NONE of the guest's FB
> ctx buffers, and there is no second crossing — only the dummy-page obligation to the guest kernel."*

The **antecedent is true** (§3). **Neither consequent is.**

- *"only the dummy page"* — refuted (`rs:docs/design/gpu_promote_ctx.md:883-892`). The guest kernel
  checks **SIZES** from static-info controls (`ogkm-580: …/gr/kernel_graphics.c:1754-1756`) and its
  **own memdesc pointers** (`kernel_graphics_context.c:1630-1631`). It never reads a ctx buffer's
  contents, never checksums one, **never touches its VA**. A backed non-faulting page at the GR VA
  satisfies nothing, because the check was never about a page.
- *"no second crossing"* — refuted by `mode2_dataplane_decision.md:74-79` (status **SETTLED**) and by
  shipped code at `C:src/qemu/nvkvm_gpu_emul.c:3902` and `:8497`.

★ The inference fails by crossing from *the host's* VAS to *the guest's* obligations without a
warrant. **Two different address spaces hold two different buffers with the same name.**

### ⊘⊘ R2 — `st=0x51` is NOT evidence of self-mapping. It is `NV_ERR_NO_MEMORY`.

There is **no "already mapped" status in the ABI**, and `0x51` is not one. The C repo reads the same
code two incompatible ways:

- `C:src/qemu/nvkvm_gpu_emul.c:7934-7937` — *"st=0x51 (NV_ERR_NO_MEMORY) on a FIXED map ⇒ the VA is
  ALREADY mapped in the host VASpace (host RM self-promoted its GR ctx at the same VAs)"*
- `C:docs/design/mode2_userbuf_vidmem_passthrough.md:237,251,270` — *"the host RM refusing a BAR1
  mapping it has no aperture for"*, root-caused by byte-count to **256 MiB BAR1 exhaustion**.

And a FIXED `map_dma` returns it for **any** occupancy including our own (`:8377-8380` calls
`back_and_map_sys` *"idempotent (st=0x51 ALREADY-MAPPED)"*; `:2046,:2062` attribute `0x51` to our own
stale CTX1 pins).

⇒ `0x51` is a **collision-or-exhaustion signal**. It cannot distinguish *"the host self-promoted
here"* from *"we already mapped here"* from *"the host is out of BAR1"*.
⚠ **The 2026-06-05 ruling's stated evidence for self-mapping is not evidence.**
★ The conclusion survives anyway on independent source (§3) — **right answer, wrong reason.**
⊘ **Do not re-cite `0x51` for it.** The rider that does *not* survive is §3's VA claim.

### ⊘ R3 — "the host self-maps there, so leave those VAs alone" — the C's own fix went the other way

`nvkvm_m2_cvas_get` (`:7723-7729`) allocates a **fresh nvkvm-owned `FERMI_VASPACE_A`** per
`(client, TSG)` and substitutes it into the GR TSG's `hVASpace`, expressly *"so every guest VA places
into a VAS WE fully control, killing the host-RM-self-promote collision (st=0x51 / Xid 32)"*;
`populate_cvas` (`:8849-8856`) restates it. ⇒ In the C **as shipped**, the host does *not* self-map
into the VAS the guest's channel runs in, because **we replaced that VAS**. *"Leave the 0x51 VAs to
the host"* describes the **broken** configuration, not the working one.

### ⊘ R4 — my seed-copy worry is dissolved by mechanism; two different gaps are real

`back_and_map` is a **double-mmap**, not a copy: the overlay at `:7946-7949` makes
`nvkvm_fb_host_overlay` (`:1258-1275`) resolve every later guest FB access at that phys to the **same
host vidmem `qva`**. `copy_content` is a one-time establishment bridge. **No steady-state
divergence.** What *is* real:

1. **VA→GPA re-binding** — `#12 STALE-SYS` (`:8396-8445`): the guest tears down and re-creates at the
   **same VA with fresh pages**; the host VAS still targets the old page; completions land in dead
   memory and the stray writes trip guest UVM's `MAX_JUMP` assert. Fixed by free-pin-and-re-back
   (`reback` capped at 64). ⚠ **Sysmem only. The FB path has drop-on-free (`:2003-2021`) and no
   re-back.**
2. **The `0x51` branch seeds nothing and overlays nothing**, deliberately (`:7887-7889`, *"a 0x51 must
   NOT be overlaid or we'd shadow the host's real buffer with zeroed memory"*; enforced at
   `:7940-7950`, both the `memcpy` and the `m2_fbback` registration inside `if (phys && ok)`). So in
   exactly the collision case the two memories diverge permanently. ★ **This is the only place my
   original worry lands — and it is an acknowledged, unclosed gap.**

---

## §2 THE 06-04 / 06-05 ADJUDICATION — the actual deliverable

| date | source | claim |
|---|---|---|
| 06-04 | `mode2_dataplane_decision.md:74-79` (**SETTLED**) | back the guest's GPU-VA for the GR golden context with **host vidmem** via `RM_MAP_MEMORY_DMA`; one-time |
| 06-05 | `mode2_doorbell_chid.md:347-353` (owner) | the host self-maps on the `0xc7c0` forward; we owe **only** a dummy page |
| 06-14 | `mode2_userbuf_vidmem_passthrough.md:86` | `back_and_map` *"called today only with labels `ctx*`/`gpfifo`/`pushbuf`/`userd`"* — 06-04's shape, built |

**They answer different questions and both are true; the ruling's *"only"* is what fails.**

- 06-04 §5: *who provides the memory the **GUEST's** promoted GR-VA resolves to, in the VAS the
  guest's channel runs in?* → host vidmem, one-time.
- 06-05: *who builds the GR context the **HOST ENGINE** runs on?* → the host RM, on the `0xc7c0`
  forward. **Correct** (§3).

⚠ **Forwarding `0xc7c0` does NOT retire 06-04's mechanism**, and the C measured why —
`mode2_dataplane_architecture.md:493-497`: *"the GR engine (FECS) writes the golden image + status
into RM-INTERNAL PRIVILEGED ctx buffers … The host writes its own buffers; the guest's copies
(`0x2efbaf000` / `0x2efa6xxx`) **remain blank**."* ★ **The host's self-mapped set is exactly the set
the guest cannot see.** The only way the host's FECS writes bytes the guest reads is if the guest's
VA — in the VAS the host channel actually runs — resolves to an object **we** placed. That is 06-04 §5.

⇒ **06-04 stands. 06-05 narrowed correctly on the host axis and over-concluded on the guest axis.**
Both are built; neither is only a plan.

---

## §3 Q1 — Does the host GR context self-map? YES, from source, independent of `0x51`

`kgrobjConstruct` → `kgrctxShouldManageCtxBuffers_HAL`
(`ogkm-580: …/gr/kernel_graphics_object.c:207,213`) → `kgrctxAllocCtxBuffers` (`:216`) +
`kgrctxMapCtxBuffers` (`:219`).
`kgrctxShouldManageCtxBuffers_KERNEL` = `gpuIsClientRmAllocatedCtxBufferEnabled(pGpu) &&
!IS_GFID_VF(gfid)` (`kernel_graphics_context.c:2599-2607`), and
`ogkm-580: …/gpu/gpu_registry.c:146-155` sets that flag `NV_TRUE` **unconditionally for every
`IS_GSP_CLIENT` GPU** — which the host's own 580.159.04 open module is.
`kgrctxMapCtxBuffers_IMPL` (`:1591-1680`) maps MAIN, PATCH, PM and the priv-access maps into
**`pKernelChannel->pVAS`**, from **host** memdescs.

⇒ Forwarding `0xc7c0` makes the host RM **allocate AND map its own** ctx buffers in the host
channel's VAS out of host vidmem. **No part is expected from us. The host GR engine needs none of the
guest's FB ctx pages.**

Measured: `rs:traces/guest_boots/run_w222_346921b_gate_qemu.log:35,37,…,49` — 8 ×
`class=0xc7c0 … → FORWARDED engine=GrCompute host_object=0xcafe00.. materialized_channel=true`.
**BUILT-AND-RAN.**

⚠ Two riders that are **NOT** established:

- *"at the SAME deterministic GR VAs (`0x120020000…`)"* — the citation chain
  (`rs:gpu_promote_ctx.md:36-40` → `rs:execution_plane.md:209-217` →
  `C:mode2_dataplane_architecture.md:416-421`) bottoms out in `st=0x51`, **refuted as evidence by
  R2**. `kgraphicsMapCtxBuffer` takes a VAS-heap address, not a fixed one. ⇒ **Treat VA coincidence
  as unverified.**
- It says nothing about the **submission** plane. Across `w218→w220→w221→w222` doorbells are
  **191/183/8 unchanged**, `forwarded doorbells 0`, `CUP2_RC=TIMEOUT`;
  `Route::NotACopyEngineChannel` refuses every `GrCompute` doorbell above the forwarding plane.
  ⇒ **The host holds eight real GR contexts and not one guest doorbell reaches any of them.**

---

## §4 Q2 — What the guest kernel's promotion actually requires of us

The guest is a GSP client ⇒ `bClientRmAllocatedCtxBuffer = NV_TRUE` ⇒ it **allocates, maps and
promotes its own** GR ctx buffers **by construction**, not optionally.

**The two apparently-conflicting statements are about different checks. Only the word *"only"* is
wrong.**

1. **The kernel-check axis — what is owed is an RPC ANSWER, not memory.** `GPU_PROMOTE_CTX
   (0x2080012b)` must return `NV_OK` or the `AMPERE_COMPUTE_B` alloc fails outright — no retry, no
   degradation (`kernel_graphics_object.c:224-225` → `kgrobjConstruct_IMPL:353-360`). The dummy page
   is refuted because the checks are on **sizes and the guest's own memdescs**.
2. **The content axis, which the ruling omits.** The guest's poll #2 consumes ctx **CONTENT** at
   guest-FB `0x2efa6xxx`, ~2225 resolves (`mode2_dataplane_architecture.md:487-491`). And what the
   guest promotes **is guest FB**: the 560-byte capture (`C:src/qemu/mode2_initctrl_ga106.h:3279-3315`)
   gives bufIds 0/2/10/11 at phys `0x2ef946000` / `0x2efa40000` / `0x2ef820000` / `0x2eed80000` —
   vidmem — plus bufId 9 `FECS_EVENT` at **sysmem** `0x107900000` (`physAttr 0x5` COH_SYS), which
   belongs to the **first** crossing.

⇒ Neither a dummy page nor privileged mirroring: **a real blank host vidmem object at the guest's FB
GPA, FIXED-mapped at the guest's VA into a VAS we own, into which the host FECS then writes the
golden context** (`mode2_cuctxcreate_resume.md:138-143`).

---

## §5 Q3 — What the C built and ran, in two generations

**GEN-1** — `nvkvm_m2_back_and_map(..., copy_content=false, "ctx%d")`, `:3892-3906`.
**BUILT-AND-RAN.** One-shot (`m2_exec_done`), gated `m2exec && m2_gr_client`, iterates the
PROMOTE_CTX-snooped `va_map[]` (`nvkvm_record_va_map`, `:2416-2438`), **skips sysmem** (`:3897`).
Measured: `mode2_cuctxcreate_resume.md:153` — *"M5.7 EXEC backed **3/6** FB working-set buffers."*
It backed half; the other three are the `0x51` class of R4-gap-2.

**GEN-2 — the one that cleared the wall** — `populate_cvas` → `pt_enum` → `leaf_flush` →
`gpga_obj_ex`, `:8456-8508`, into the fresh per-`(client,TSG)` VAS from `cvas_get`.
**BUILT-AND-RAN.** Vidmem leaves get a **blank host vidmem object, double-mmapped** — *"the guest
manages its contents and the host GPU fills the golden ctx on execution — both sides share ONE
coherent host object"* (`:8456-8462`). Establishment bytes preserved **unconditionally** (`:8282-8290`),
not behind a flag.

⇒ 06-04 §5 shipped, in gen-2 form. ★ `mode2_userbuf_vidmem_passthrough.md`'s *"implementation
pending"* refers to the **user-buffer** extension (*"User allocations are not snooped today"*),
**not** to the `ctx*` backing — which its own machinery table heads *"already proven"*.

**Staleness ledger.** Steady state: safe by double-mmap. Re-binding: **sysmem only**. Collision:
`0x51` ⇒ no overlay, no seed, permanent divergence. **The C admits the gap in comments and did not
close it for FB.**

---

## §6 Q4 — Is it on the green path? YES — and the trap here is a COMMENT, not a flag

- **`m2exec` default is `true`** — `:9929`, *"(always on)"*; field comment `:542` *"default ON;
  debug-only off"*.
- ⚠ ★★ **The comment at the ctx call site is STALE and says the opposite** — `:3884`, *"gated
  m2exec, default off"*. **Anyone auditing by grepping for "default off" would wrongly quarantine a
  live path.** ⇒ This is the inverse of the campaign's usual trap: not a default-off path that never
  ran, but a **default-on path documented as off**.
- `scripts/mode2_diag/bench_boot.sh:56` sets only `NVKVM_M2CEFWD=1` and never `NVKVM_M2EXEC_OFF` ⇒
  **the green `cup8`/LLM boots ran with `m2exec` ON.** The only off-switch use is the deliberately
  hermetic capture (`rec_capture.sh:58`).
- Default-OFF and **not** gating this crossing: `m2hostsem`, `m2cexec`, `m2opaque`, `m2trace`,
  `m2romregs` (`:9930-9935`).

---

## §7 THE PARTITION-RULE VERDICT — which side of the line the GR ctx buffers fall on

Rule (`mode2_dataplane_decision.md:45-56`): decided by whether the guest kernel maps the buffer into
guest **userspace**. ⊘ Note §3's *"trap"* means *"not on the per-access hot path"*, **not** *"fake
it"* — §5 puts a kernel-only buffer in the trap bucket **and** backs it with real host vidmem.

| buffer | flag | bucket | verdict |
|---|---|---|---|
| MAIN | `MEMDESC_FLAGS_GPU_PRIVILEGED` (`:1127`) | kernel-only | trap → **real host vidmem, one-time** |
| PM | `MEMDESC_FLAGS_GPU_PRIVILEGED` (`:1277`) | kernel-only | same |
| PRIV_ACCESS_MAP / UNRESTRICTED | privileged | kernel-only | same |
| **PATCH** | ⚠ **`MEMDESC_FLAGS_OWNED_BY_CURRENT_DEVICE` only** (`:1206`) — **NOT privileged** | **not kernel-private** | **must be REAL by the userspace-visibility test** |
| FECS_EVENT | sysmem, COH_SYS | — | the **first** crossing |

★★ **PATCH is the sharpest line here, and it was already written down**
(`rs:docs/design/gpu_promote_ctx.md:904-906`): it is on the mandatory promote list for a compute
object (`ogkm-580: kernel_graphics_object.c:594-602`, `promoteNon3d[]`), and it is the one GR ctx
buffer the *"guest userspace should see almost no fake pages"* directive reaches directly.
⇒ **The GR ctx set is NOT uniformly ours to fake. MAIN/PM/priv-maps are ours to back one-time;
PATCH is passthrough-or-nothing.**

---

## §8 NAMED CONSTRAINT — re-verified at 580.159.04, still holds

**We cannot ask the host where its GR buffers are.**

- `NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO (0x20801219)` — `generated/g_subdevice_nvoc.c:5431-5433`,
  `flags = 0x8000` = `RMCTRL_FLAGS_CPU_PLUGIN_FOR_LEGACY` **only**. Neither `NON_PRIVILEGED (0x8)`
  nor `PRIVILEGED (0x4)` is set ⇒ default `RMCTRL_FLAGS_KERNEL_PRIVILEGED = 0x0` ⇒ **kernel clients
  only**.
- `NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR (0x410103)` — `generated/g_mem_nvoc.c:112-114`,
  `flags = 0x4` = `RMCTRL_FLAGS_PRIVILEGED` ⇒ **admin only**.
- Both ⇒ `NV_ERR_INSUFFICIENT_PERMISSIONS = 0x1B`, matching the C's 2026-06-04 measurement.

⇒ **Forecloses every design that learns host GR buffer layout or host-phys.** Both surviving shapes
are VA-based and unprivileged — which is exactly why `RM_MAP_MEMORY_DMA` FIXED is 06-04 §5's
primitive: `NV_ESC_RM_MAP_MEMORY_DMA` is `NV_CTL_DEVICE_ONLY` but **not privilege-gated**.

---

## §9 What the next rung inherits

- The second crossing is **scoped, decided and twice-built**. Not new work to design — work to
  **port**. ★ And its C form depends on a **fresh nvkvm-owned VAS** (R3), not on respecting host
  self-maps.
- Three narrow unresolved items: (i) **no FB re-back** for VA→GPA re-binding (sysmem has one);
  (ii) the **`0x51` branch** seeds and overlays nothing; (iii) **PATCH's non-privileged status** has
  never been acted on separately from the privileged buffers.
- ⚠ **Do not open the `GrCompute` doorbell gate merely to make a falsifier fire** —
  `rs:docs/design/gpu_promote_ctx.md:1097-1099` records the hostile-guest boundary at exactly that
  line. **Context allocation and ring submission are two rungs; w222 moved only the first.**

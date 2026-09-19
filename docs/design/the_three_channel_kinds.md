# The three channel kinds — passthrough, translated, emulated

**STATUS: DESIGN, 2026-09-19 (w803). Owner ruling. Not yet implemented.**

> **Owner:** *"I think for the second case `SRC/DST_TYPE = PHYSICAL` we can create a new
> channel type beyond emulated and passthrough. Named virtual."* … *"emulated remains for
> channels that need a real function implementation like the UVM kernel one or the ones we
> need to stub things. these can still use scratchpad like for refresh"*

## 0. Why two kinds was the wrong number

`GuestChannelKind` has been `{Emulated, Passthrough}`, decided by *who allocated the channel*
(`project.rs:311` — `anchor == SYSTEM_ANCHOR ⇒ Emulated`). That conflates three different
questions:

1. **may hardware run these bytes untouched?**
2. **must each ENTRY be inspected before it is safe?**
3. **is there a GPU operation that corresponds to this at all?**

⊘ A channel-level answer is the wrong granularity for (2). The owner's words: *"For channels
where you can't know per entry if its privileged/unprivileged allowed like phys, this virtual
channel mode might be a solution."* CeUtils is exactly that channel — see §2.

## 1. The three kinds

| | **Passthrough** | **Translated** | **Emulated** |
|---|---|---|---|
| who drives | unprivileged guest userspace | the guest kernel | the guest kernel |
| who moves the bytes | **the GPU** | **the GPU** | nobody — there are no bytes to move |
| do we inspect the pushbuffer | **never** | **every entry** | yes, we implement the function |
| guest's ring | the guest's own, in GPGA | the guest's own, in GPGA | the guest's own, in GPGA |
| host channel's ring | the guest's (adopted) | **ours, in scratchpad VA, OUTSIDE GPGA** | n/a / ours |
| completion the guest waits on | written by hardware | **written by hardware** | ours |
| where the work runs | host channel, guest VAS | host channel, **GPGA VAS** | VMM worker; may use the scratchpad (e.g. refresh) |
| examples | CUDA userspace channels | **CeUtils scrub, kernel CE** | **UVM kernel channel; stubs** |

★ **"Emulated" names a function we implement because no GPU operation corresponds to it** — or
a value ogkm expects to read that no engine ever produces. It does **not** mean "our CPU does
the guest's copy": §46 forbids that, and §37 already said emulated channels execute *through a
raw client the VMM owns*, i.e. on the GPU.

## 2. Why CeUtils forced the third kind — measured in ogkm, not inferred

`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:1053-1091`:

```c
if (pChannel->bUseVasForCeCopy && srcAddressSpace == ADDR_FBMEM) {
    srcAddr = srcAddr + pChannel->fbAliasVA - pChannel->startFbOffset;
    retVal |= DRF_DEF(B0B5, _LAUNCH_DMA, _SRC_TYPE, _VIRTUAL);
} else {
    retVal |= DRF_DEF(B0B5, _LAUNCH_DMA, _SRC_TYPE, _PHYSICAL);
}
```

⇒ NVIDIA already implements the identity-mapped-FB idea, and its own log line names it:
*"CeUtils VAS : FB (addr, size) **identity mapped to VAS** at addr, page size"*
(`mem_utils_gm107.c:505-516`). Format rules, stated there: FB base **and** VA base must be
**512 MiB aligned**; the page-size mask is capped by the alignment of each and by the VMMU
segment size; `RM_PAGE_SIZE_512M` and `RM_PAGE_SIZE_256G` are excluded.

⊘⊘ **But the guest will not choose it on anything we currently target.**
`bUseVasForCeCopy` comes from `NV0050_CEUTILS_FLAGS_VIRTUAL_MODE` (`ce_utils.c:234`), which
`memmgrInitCeUtils` sets **only** when `bCePhysicalVidmemAccessNotSupported`
(`mem_mgr.c:4129`) — which is `gpuIsSelfHosted(pGpu)` (`mem_mgr.c:373`) — which is set **only**
in `kern_gpu_gh100.c:417,426`, i.e. self-hosted **Hopper**. On GA10x/AD10x/TU10x the guest
always emits `PHYSICAL`.

⚠ **And a chip table could not express this even for one part.** A second emitter,
`_ceChannelPushMethodsBlock_GM107` (`mem_utils_gm107.c:2098-2100`), hard-codes
`SRC_TYPE=_PHYSICAL`/`DST_TYPE=_PHYSICAL` and **never consults the flag**. One driver, one
part, two paths, two answers.

⇒ **The discriminator is the operand type in the bytes, never the chip.** That is
`derive_per_die_maintain_per_family` and `a_capture_derived_table_expires_as_a_vendor_regression`
applied before the table exists rather than after it rots. A future part that flips to virtual
mode is promoted to Passthrough automatically, with no edit.

## 3. ⊘ Why a PHYSICAL operand may never simply pass through

`LAUNCH_DMA.SRC_TYPE=_PHYSICAL` **bypasses the MMU**. A guest-authored physical address
arriving at the engine unmediated is a **host** physical address. That is not the
guest-internal isolation question §45 defers to ogkm — it is the escape class, and it is why
these channels cannot be made passthrough by relaxing a check.

## 4. How a Translated channel runs

1. Read the guest's USERD. If `GP_PUT` advanced, advance `GP_GET` and read the ring.
2. Decide where the entry's pushbuffer lives:
   - **sysmem** ⇒ a **bounded** copy into our own byte array (`unsafe` with the bound checked;
     the guest's GPA window is the bound).
   - **vidmem** ⇒ a **CE copy** on the scratchpad's existing raw client. ⚠ Asynchronous, so it
     belongs on the worker's `epoll` loop (§35, §37(b)).
3. Translate the copied pushbuffer: aperture `PHYSICAL → VIRTUAL`, operand addresses rebased
   into the GPGA VA space. ⊘ Zero arithmetic in the common case: **VA == FB offset**, because
   the single store makes the framebuffer address the object offset.
4. Submit it on the host channel. **The semaphore release is translated like any other operand
   and left in place** — we never produce its value.
5. Entries batch: each guest entry becomes one host entry, in order, on one host channel.

### The two properties that make this correct, not merely convenient

★★★ **(a) The VAS is the bound.** The GPGA VA space maps *only* the store, so a translated
operand **cannot** name memory outside the guest's own. Containment is a property of the
address space we submit into — there is no validation to forget and no bound to pass around.
Contrast `a_bound_on_reads_is_not_a_bound_on_emits`, where the bound had to be carried.

★★★ **(b) The completion stays hardware's.** The guest spins on a word a **real engine**
writes. We translate the *address* of the release; we never produce the *value*. That is the
line between this and what the C did (`citing_the_c_where_it_forges`) — a completion we cannot
forge is a completion we cannot get wrong.

⊘ And it is already §39(a)-shaped — **copy, then check, then use**. The guest mutating its own
pushbuffer after step 2 changes nothing, because we act on our copy. It also sits on the right
side of §37's line against §20: a linear scan over a **copied** buffer chases no guest-authored
structure, so it runs in the VMM worker, not the scratchpad.

### Interrupts

As passthrough: registering an interrupt vector arms delivery after completion, or immediate
delivery if it has already completed — through the eventfd the raw client yields (§37(b)).

## 5. What must be decided before implementing

1. ⚠ **`GP_GET` acquires a stated divergence.** On hardware it advances when the **engine**
   fetches; here when **we** fetch. Drivers wait on the semaphore, so this is safe — but it
   must be written down, or someone will infer "the engine finished this entry" from it.
2. ⚠ **A scoping rule, or the launch floor returns.** The per-entry copy is real cost
   (`w315` measured **86.7 ms/launch** when per-launch work ran inline). Translated is for
   channels that need per-entry decisions. Hot userspace channels stay Passthrough.
3. ⊘ **Untranslatable entries refuse BY NAME** — an operand naming sysmem we have not pinned,
   an aperture we do not model. Never a silent skip
   (`refuse_by_name_means_the_name_is_true`).
4. **Sysmem operands** (`SET_SRC_PHYS_MODE_TARGET_{COHERENT,NONCOHERENT}_SYSMEM`) are not
   covered by a store-only VAS. Unmeasured: how often CeUtils names them.
5. **The Hopper+ fast scrubber** (`hTdCopyClass >= HOPPER_DMA_COPY_A && !bUseVasForCeCopy ⇒
   FAST_SCRUBBER_CHANNEL`, `ce_utils.c:245`) is a third method stream, relevant as soon as
   "all GPUs" includes Hopper/Blackwell.
6. **The name.** `Virtual` collides four ways in this tree — `LAUNCH_DMA.SRC_TYPE=_VIRTUAL`,
   `NV01_MEMORY_VIRTUAL`, `FERMI_VASPACE_A`, and the VA space itself. `channel_kind.rs:78-80`
   already carries a note justifying why `Passthrough` does *not* collide. **`Translated`** says
   what it does and collides with nothing.

## 6. The type-level work, and it is a feature

`GuestChannelKind::ALL` is `[GuestChannelKind; 2]` and `trap_contract()` / `hosted_by()` match
it exhaustively with **no `_` arm** — deliberately: *"a new variant fails this build until
somebody says what the shell does with it"*. Adding the third forces every site to answer,
which is how we find the places that silently assumed two.

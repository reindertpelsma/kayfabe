# The instance-block oracle — investigated, and REFUSED

**Status: closed as a NEGATIVE.** ⊘ Do not re-propose it. `execution_plane_increments.md` §16.28.13
listed this as *"checked and named, not done"* and owed an instrument; this document discharges that
debt by showing the instrument **cannot answer the question it was proposed for**, and by naming the
two things worth building instead.

**Verdict: (b), and harder than (b).** Not merely *"it will probably read empty"* — the channel
instance block in our framebuffer is **guaranteed** to hold a genuine, guest-written **zero**, because
the guest's own CPU-RM explicitly zeroes it and the **only** code in the driver that ever writes its
page-directory base lives in the physical RM we replace (§3.1: `ogkm-580:
src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:267-271` zeroes it; `ogkm-580:
src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:2118-2119` is the only writer of its page-directory base, and
has no caller under `gpu/fifo/`). ⇒ The read succeeds, the page is resident, the value is real, and the
number is wrong. That is the worst shape an oracle can have: it does not fail, it agrees with nothing
while looking exactly like a sound reading. `[src]`

---

## 0. ★★★★ WHAT THIS REFUTES — starting with my own brief

### 0.1 The brief's central premise is false

> *"the object model is BOOKKEEPING; the instance block and the page tables are the MACHINE. Hardware
> never sees VASpace objects — the host engine walks from the page-directory base in the channel's
> INSTANCE BLOCK. ⇒ **That is a physically independent source of the same number.**"*

The first two sentences are **true of hardware** and remain worth keeping. The inference is **false of
this device**, and the word that breaks it is *source*.

An instance block is a **source** only where something wrote it. `[src]` On a GSP client, the writer of
the channel instance block's `PAGE_DIR_BASE` is **physical RM, inside GSP firmware** — the component
kayfabe *is*. In our system the field has no writer at all. ⇒ **Reading it is not a second derivation;
it is reading back our own silence.** And if we close that gap the way the C's `#12` cont.5 proposed —
*"making us populate what real GSP populates"*
(`C: docs/design/mode2_2nd_context_hang.md:565-568`) — the read becomes a derivation of the
publication **through us**, i.e. exactly the *"two of our own computations agreeing"* failure the brief
was written to avoid. The oracle is a **tautology in both directions**: empty before we write it,
circular after.

### 0.2 §16.28.13's hedge is refuted, and it was mine

`docs/design/execution_plane_increments.md:8542-8545` says:

> ⚠ **But that negative is scoped to GSP-managed channels, and nobody has asked it of this
> Device-parented one.**

⊘ **Wrong, and wrong in the direction that keeps an item alive.** The C's own record already answered
it at the level of *mechanism*, not of sample:

> *"the instblk at `0x2efa6e000` is simply **never populated** — every channel logs
> `M5.14 … PDB empty (GSP-managed)` because our fake GSP (which owns instblk construction in
> GSP-client mode) never writes RAMIN+0x200. So **"read the instblk PDB" is a dead end for *all*
> channels, not just this one.**"*
> — `C: docs/design/mode2_2nd_context_hang.md:463-469` `[measured]`

`GSP-managed` in the C's log string is a **diagnosis of who was supposed to write it**, not a
**restriction on which channels it applies to**. §16.28.13 read the parenthetical as a scope and
manufactured an open question out of a closed one. ★ `a_wrong_citation_is_more_durable_than_none`,
in its subtlest form: the citation was to real text, the text said the right thing, and the *reading*
narrowed it.

⇒ **The parenthetical in a log message is not the scope of the finding.** §3.1 below re-derives the
same conclusion from the guest's source, independently of the C, so it no longer rests on a log string
at all.

### 0.3 Two smaller refutations of my own working notes

- ⊘ **"kayfabe cannot read sysmem"** — I believed this after reading
  `crates/kayfabe-device/src/ceresolve.rs:175-180` (*"This port has no query for the guest's RAM
  extent"*). That is a statement about a **bound**, not about **access**: the very same paragraph says
  `kayfabe_gsp::GuestRam` *"exposes only read/write"*, and `crates/kayfabe-gsp/src/ram.rs:51-65` is a
  live read/write trait over guest physical memory. **Both apertures are reachable** (§2). `[src]`
- ⊘ **The C comment `nvkvm_gpu_emul.c:225` calls `chan_inst_block` *"unused"*, and it is not.**
  `nvkvm_gpu_emul.c:5873-5893` reads it on every failed content-pick, and `:5334-5336` and `:5449-5455`
  both consume the result. The field is *ineffective*, which is a different word. ★ `a_wrong_comment_is_why_nobody_looked` — a
  reader who trusted `:225` would have concluded the C never tried this, when in fact the C tried it,
  logged it, and root-caused the failure.

---

## 1. Q1 — do we capture `instanceMem.base` and its aperture? **No, and nothing reads it.**

### 1.1 The C does capture it, at a cross-checkable offset `[src]`

`C: src/qemu/nvkvm_gpu_emul.c:2877-2878`, inside the `fn == 103` (`GSP_RM_ALLOC`) snoop that also
yields `gpFifoOffset`:

```c
s->chan_inst_block = ldq_le_p(cmd + 256);        /* instanceMem.base */
s->chan_inst_sys   = (ldl_le_p(cmd + 272) == 1u); /* ADDR_SYSMEM */
```

★ The offsets corroborate against a **second, independent site in the same file**: the `M5.3 DIAG`
dump at `C: :7038-7048` names `inst.base@144` and `as@160` *within the params block*, and the RPC body
puts `params` at `cmd + 112` (`C: :2701-2703`). `112 + 144 = 256` ✔ and `112 + 160 = 272` ✔. Both
agree with the header: `NV_MEMORY_DESC_PARAMS instanceMem` is the field after `hObjectEccError`
(`ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:323`), and
`kayfabe_abi::submit`'s own layout map places it at `+144`
(`crates/kayfabe-abi/src/submit.rs:264`). **Three sources, one number.** `[src]`

### 1.2 kayfabe parses neither field, and the one mention is a comment

`grep -rn "instance_mem\|instanceMem\|inst_block\|instance_block\|instblk" --include=*.rs`
over `/workspace/nvkvm-rs` returns **exactly one hit**: `crates/kayfabe-abi/src/submit.rs:264`, a line
inside the ASCII layout table in `ChannelAllocParams`' rustdoc. ⊘ No decoder, no field, no caller.
`[measured 2026-08-09, bare `grep -rn`, `rc=0` with the hit — the search ran]`

Note *which* struct that is: `ChannelAllocParams` is an **encoder** — the isolate's outbound channel
alloc — and its own rustdoc says *"Everything from +144 on is `// reserved` in the header and is left
zero"* (`submit.rs:273`). So the single mention is in the direction we **write**, not the direction we
**observe**. The inbound decoder is `ChannelAllocFacts`
(`crates/kayfabe-abi/src/view.rs:426`), and it stops at +32.

### 1.3 What a parse would cost: a version-dispatched reader, and the pattern already exists `[src]`

`crates/kayfabe-abi/src/versions.rs:1496` — `CHANNEL_ALLOC_PREFIX = 32`. `instanceMem` at +144 (580)
is far past it, in the region 610 shifts by eight bytes (`view.rs:415-421` records the same skew for
`engineType`: `+128` at 580, `+136` at 610). ⇒ A prefix-contract decoder **cannot** read it.

⊘ That is not a blocker; it is a solved problem with two precedents in the same file:

| field | wire type | decoder | unread-boundary answer |
|---|---|---|---|
| `errorNotifierMem` | `ChannelNotifierWire` | `decode_channel_error_notifier` (`versions.rs:1023-1031`) | `Ok(None)` |
| `hUserdMemory[0]` / `userdOffset[0]` | `ChannelUserdWire` | `decode_channel_userd` (`versions.rs:1050-1055`) | `Ok(None)` |

A third row — `ChannelInstMemWire` → `decode_channel_inst_mem` returning
`Ok(Option<(u64 /*base*/, u32 /*addressSpace*/)>)` — is a **table edit plus ~40 lines**, and inherits
the correct three-valued semantics for free: `None` means *this port has not read that version's tree*,
never *the channel declared none*. ⚠ That distinction is the one
`accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance` exists for, and it is already enforced by
the pattern.

**⇒ Q1 answer: not captured, not read, and cheap to capture — but see §5 for what it is worth
capturing *for*, which is not the oracle.**

---

## 2. Q2 — could we READ the instance block? **Yes, in both apertures. Reachability is not the
blocker.**

The alloc params declare the aperture (`instanceMem.addressSpace`), so there is no guessing:

| declared aperture | our reader | reachable? | cost |
|---|---|---|---|
| `ADDR_FBMEM` (vidmem) | `FbStore::read` (`crates/kayfabe-device/src/fbwin.rs:241`) | **yes** | plumbing only — a `&mut dyn FbStore` at the site that holds channel facts |
| `ADDR_SYSMEM` (guest RAM) | `GuestRam::read` (`crates/kayfabe-gsp/src/ram.rs:58`) | **yes** | plumbing only — a `&mut dyn GuestRam` at the same site |

Offsets come from the header, read here: `NV_RAMIN_PAGE_DIR_BASE_LO` is word 128 = byte `0x200`,
`_HI` word 129 = `0x204`
(`ogkm-580: src/common/inc/swref/published/pascal/gp100/dev_ram.h:49-50`), and the C's constants
`NVKVM_RAMIN_PDB_{LO,HI}_OFF` (`C: src/qemu/mode2_regs_ga10x.h:134-135`) agree with them. ⊘ The C
*records* having re-checked them against the same manuals
(`C: docs/design/mode2_2nd_context_hang.md:463-466`); that is a claim in its ledger read by me, not a
check re-run here. `[src]`

★ **Existence proof that the whole mechanism works in this codebase's C predecessor**: the C reads a
*different* instance block — BAR2's — through the identical offsets and gets a **real answer**
(`C: nvkvm_gpu_emul.c:4792-4797`). §3.2 explains why that one is populated and channel ones are not;
the contrast is the argument.

### 2.1 ⚠ But the reader cannot report "unmeasured", and that is a defect of *our* memory plane

`SparseFb::read` fills **zeros and returns `Ok`** for a page nobody wrote
(`crates/kayfabe-device/src/fbwin.rs:654-668`, and the module argues for it at `:560-564`). ⇒ A naive
`read_u32(instblk + 0x200)` **cannot distinguish** *"never written"* from *"written, and it is zero"*.
That is `c_oracle_empty_rows_are_wrong` reproduced inside our own device: ⊘ **an empty read is evidence
of nothing.**

The distinction *is* recoverable — `FbStore::page_origin` (`fbwin.rs:329`) returns
`Option<FbPageOrigin>`, so residency and the writer tag are queryable per page — and any honest
instrument here **must** use it rather than the raw read.

⊘ **And it still would not save the oracle**, which is §3.3: the guest zeroes the page itself, so it
*is* resident, it *was* written, and `page_origin` will confirm a real write of a real zero.

**⇒ Q2 answer: reachable in both apertures; the blocker is not reachability.**

---

## 3. Q3 — the counter-evidence, sourced from the guest driver rather than from a log string

### 3.1 ★★★★ THE MECHANISM, in five citations, all at the bench driver `ogkm-580: 580.159.04`

The brief asked *"which channels was it empty for"*. The right question turned out to be *"who is the
writer"*, and the driver answers it without ambiguity.

1. **The guest CPU-RM takes the allocate path, not the describe path.** On a GSP client,
   `RMCFG_FEATURE_PLATFORM_GSP` is 0 in the guest kernel module, so the dispatch falls to the third
   arm, whose own comment names the case:
   *"On baremetal, **GSP client**, or SRIOV host, alloc mem"* → `kchannelAllocMem_HAL`
   (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2330-2339`). The `_FromParams` arm
   above it (`:2323-2329`) is the **GSP-side** one, and `_kchannelDescribeMemDescsFromParams`
   asserts exactly that: `NV_ASSERT_OR_RETURN((RMCFG_FEATURE_PLATFORM_GSP && !bGspOwned) || …)`
   (`:2392`). `[src]`

2. **★★★ That path allocates the instance block and EXPLICITLY ZEROES IT.**
   `kchannelAllocMem_GM107` (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_channel_gm107.c:163`)
   creates the memdesc, allocates it, and then:

   ```c
   // Initialize the instance block of the channel with zeros
   status = memmgrMemDescMemSet(pMemoryManager, pInstanceBlock->pInstanceBlockDesc, 0,
                                TRANSFER_FLAGS_NONE);
   ```
   `ogkm-580: kernel_channel_gm107.c:267-271` `[src]`

3. **The guest then RPCs the base to the GSP — and only the base.**
   Guarded by `if (IS_GSP_CLIENT(pGpu) || bFullSriov)` under the comment
   *"These fields are only filled out for GSP client or full SRIOV, i.e. **the guest independently
   allocs ChID and instmem**"* (`ogkm-580: kernel_channel.c:2703-2707`), the CPU fills
   `pRpcParams->instanceMem.{base,size,addressSpace,cacheAttrib}` (`:2727-2734`). ⊘ It sends an
   **address and an aperture**. It never sends, and never writes, a page-directory base.

4. **★★★★ The ONLY function in the open tree that writes an instance block's `PAGE_DIR_BASE` is
   `kgmmuInstBlkInit_IMPL`** (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:2040`), whose two
   write sites are
   `MEM_WR32(pInstBlk + dirBaseHiOffset, dirBaseHiData); MEM_WR32(pInstBlk + dirBaseLoOffset, dirBaseLoData);`
   at `:2118-2119` and `:2152-2153`. `[src]`

5. **★★★★ It has ZERO callers under `gpu/fifo/`.**

   ```
   $ grep -rn "kgmmuInstBlkInit" src/nvidia/src/kernel/gpu/fifo/      → rc=1   (no hits)
   $ grep -rn "kgmmuInstBlkInit(" src/ | grep -v "\.h:"               → rc=0   (8 hits)
   ```
   `[measured 2026-08-09 in `research_clones/ogkm-580.159.04`, bare `grep -rn`, both exit codes
   recorded; `rc=1` is a genuine zero-hit and not a `timeout`-mangled 127, and the second command is
   the positive control that proves the pattern matches at all.]`

   The eight callers are, in full — and every one was opened, not counted:
   `fabric_vaspace.c:113,139,187`, `kern_bus_ga100.c:313,504` and `kern_bus_gh100.c:2258` are all
   **FLA** (each sits under a literal *"Instantiate Inst Blk for pFlaVAS"* / *"for FLA"* comment);
   `kern_hwpm_streamout.c:531` is **HWPM streamout**; `kern_bus_gm107.c:5072` is **BAR1**
   (*"Initialize the instance block VAS state"*, on `pKernelBus->bar1[gfid].pInstBlkMemDesc`).
   ⇒ **Every instance block the guest CPU-RM initialises through this function is a BUS or ENGINE
   instance block. Not one is a channel's.**

   ⚠ ⊘ **And BAR2 is not in that list either** — a detail I got wrong on the first pass and correct
   here rather than quietly. BAR2's instance block is written by a *different* CPU-side function,
   `kbusInitInstBlk_GM107` (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:5208`),
   which stores `NV_RAMIN_PAGE_DIR_BASE_{HI,TARGET,LO}` **through the BAR0/`PRAMIN` window**
   (`:5237-5244`) precisely because BAR2 is not yet usable to reach its own instance block. That is
   the write the C snoops, and §3.2 rests on it. `[src]`

**⇒ The channel instance block's page-directory base is written by physical RM inside GSP firmware,
and by nothing else. kayfabe *is* that firmware. The field has no writer in our system.** `[src]`

### 3.2 The contrast that proves the reasoning, not just the conclusion

The C successfully reads a PDB out of an instance block — **BAR2's** — through the same offsets, the
same framebuffer, and the same code shape (`C: nvkvm_gpu_emul.c:4792-4797`). Why does that one work?
Because the BAR2 instance block is one the **guest CPU** writes, with its own dedicated function and
its own transport: `kbusInitInstBlk_GM107` stores the page-directory base through the BAR0/`PRAMIN`
window (`ogkm-580: kern_bus_gm107.c:5208`, writes at `:5237-5244`) — CPU stores, into our framebuffer,
which we see. The C's own comment says the same thing from the other side:
*"the GSP (which we fake) binds BAR2 from the instance block **the CPU builds in FB**"*
(`C: nvkvm_gpu_emul.c:3756-3762`). `[src]`

⇒ Same read, same offsets, same store: **populated where the CPU writes, empty where the GSP writes.**
The split falls exactly on the writer, and not at all on the channel's parentage — which is the
`Device`-parented-vs-GSP-managed distinction §16.28.13 hoped would rescue it.

⚠ ⊘ **And do not port the C's BAR2 trick.** `nvkvm_gpu_emul.c:3757-3769` *declares* any 4-byte FB write
at `(addr & 0xFFF) == 0x200` with `(val & 3) == 0 && (val & 0xFFFFF000) != 0` to be an instance-block
bind. That is reverse resolution by content — forbidden by `mode2_address_table.md` — and
`crates/kayfabe-device/src/bar2.rs:41-45` already records the refusal to port it.

### 3.3 ★★★ The failure mode is worse than emptiness — the zero is GENUINE

Compose §3.1 item 2 with §2.1:

1. The guest **writes zeros** across the whole instance block (`kernel_channel_gm107.c:267-271`).
2. Those writes reach our framebuffer, so the page becomes **resident** with a recorded
   `FbPageOrigin`.
3. The GSP write that would replace `+0x200` **never happens**, because we are the GSP.
4. Our reader returns `0` (`crates/kayfabe-device/src/fbwin.rs:654-668`), `page_origin` returns
   `Some(..)` (`:329`, populated on every write at `:711`), and every soundness check we could write
   reports the value as sound. `[src]`

⇒ The instrument does not report *unmeasured*. It reports a **confidently measured wrong number**, and
the honest residency check that §2.1 demands **certifies** it. ★ This is the
`c_oracle_empty_rows_are_wrong` failure mode with the one mitigation that usually works — *check
whether anything was written* — already defeated.

### 3.4 ⊘ Honest caveat on the C's *sample*, which the mechanism makes moot

`M5.14` fires only inside `if (s->chan_pdb == 0 && s->chan_inst_block)`
(`C: nvkvm_gpu_emul.c:5873`) — i.e. **only after the content-pick has already failed**. So the C's
*"every channel logs PDB empty"* is literally a statement about *every channel that reached the
fallback*, a subset biased toward failures. ⚠ That is exactly the
`a_correct_capture_can_answer_the_wrong_question` shape, and had §3.1 not existed it would be a real
hole. It is moot because §3.1 argues from the **writer**, over all channels and all instants, and
never from the C's sample. Recorded so nobody re-opens the question by finding the sampling bug and
mistaking it for a rescue.

### 3.5 ★★★ CORROBORATION FROM THE OPPOSITE DIRECTION — the guest asks *us* to write them

Added after `§16.29.5b` (`3686b8b`) landed mid-investigation, because it settles §3.1 from the one
angle §3.1 does not cover: not *"who writes the instance block"* but *"who does the guest **expect** to
write it"*.

`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x801813`) carries an `ALL_CHANNELS` flag whose header
documentation reads, in full:

> *"If true, RM will update the instance blocks for all channels using the VAS and ignore the `chId`
> parameter."*
> — `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:803-805` `[src]`

⇒ The guest hands a page-directory root **across the RPC boundary** and asks the RM on the far side to
propagate it into every affected channel's instance block. On a GSP client the RM on the far side is
**physical RM in GSP firmware** — us. ★ So the instance block is not merely something we fail to
observe; it is something the guest is **explicitly delegating to us to produce**, and its emptiness in
our framebuffer is the exact, predicted consequence of our not having done so.

⚠ ⊘ **One correction to §16.29.5b, offered rather than edited in** (its rung is live and this document
does not touch that file): it reads `ALL_CHANNELS` as *"naming the instance-block update that
§16.28.13 filed as 'checked and named, not done'"*. Those are two different things pointing opposite
ways. §16.28.13's item was a **read** — use the instance block as an oracle. `ALL_CHANNELS` is a
**write** obligation on us. Serving `0x801813` therefore does not deliver the oracle; ★ it removes the
last reason to want one, because the same message states `physAddress` outright (§5.3).

**⇒ Q3 answer: it was empty for ALL channels; the parenthetical `(GSP-managed)` diagnosed the
missing writer and did not scope the finding; and the guest's own source reproduces the conclusion
without the C.**

---

## 4. Q4 — the predicate, stated so it can fail

**P (INSTBLK-AGREES).** For every channel `c` whose address space this port resolves by any route,
let `pub(c)` be the page-directory root recorded for `c`'s VA space from the guest's own
`0x90f10106` publication (`crates/kayfabe-device/src/gvaspub.rs`), and let `ib(c)` be the 64-bit value
assembled from `(read(instanceMem.base + 0x204) << 32) | (read(instanceMem.base + 0x200) & 0xFFFF_F000)`
in the aperture `instanceMem.addressSpace` declares. Then:

> `page_origin(instanceMem.base).is_some()  ∧  ib(c) == pub(c)`

It is **three-valued on purpose**, and the third value is the whole point:

| outcome | reading | which side to believe |
|---|---|---|
| `ib == pub`, page resident | agreement | — |
| `ib != pub`, page resident, `ib != 0` | ★ real disagreement | **`ib`**, and loudly. The engine walks the instance block; the object model is bookkeeping. A mismatch means the publication we route on is not the root hardware would use, and every VA we translated for `c` is suspect. This is the outcome the oracle was proposed to detect. |
| `ib == 0` | ⊘ **VACUOUS — not a mismatch and not an agreement** | **neither.** Per §3.1 nothing in our system ever writes that field, so `0` is the *predicted* value and carries no information about `pub`. A predicate that fired here would fire on every channel forever; one that passed here would be passing on a constant. |

★ **And this is where the investigation ends, because §3.1/§3.3 prove the third row is the ONLY row we
can ever be in.** `P` is not false. `P` is **undefined on our entire input domain** — a predicate whose
discriminating cases are unreachable by construction.

⚠ One corollary worth keeping, because it inverts cheaply: **row 2 can still be tested for, and if it
ever fires our model of the writer is wrong.** See §5.2.

**⇒ Q4 answer: the predicate is writable and is vacuous, and its vacuity is provable in advance from
the driver source rather than discoverable by running it.**

---

## 5. Q5 — verdict, and the two things to build instead

### 5.1 ⊘ The oracle: DO NOT BUILD

**(b), sharpened.** Buildable, reachable, cheap — and it would read a genuine guest-written zero for
every channel (§3.1: `ogkm-580: kernel_channel_gm107.c:267-271` and `kern_gmmu.c:2118-2119`), certify
it as sound, and be a constant. The only way to make it non-constant is to synthesise the instance
block ourselves from the publication, at which point it is the same number compared with itself.
`[src]`

⊘ Closed. §16.28.13's *"strongest available oracle: two independent derivations of one number"* is
withdrawn: **there is one derivation, and the instance block is downstream of it or of nothing.**

### 5.2 ★ BUILD ANYWAY, for two other reasons — but capture the ADDRESS, not the CONTENT

The parse from §1.3 is still worth landing. Its value is `instanceMem.base` as an **identity**, which
the guest genuinely declares, and never `instblk[0x200]` as a **root**, which nobody writes.

1. ★★★ **It closes a named `[unmeasured]` in `simulated_gpu_fault.md`.** That doc's §5.2 reason 3 for
   deferring the fault-buffer write is *"We cannot fill it honestly. The entry's attribution key is
   the **instance block physical address**"*
   (`docs/design/simulated_gpu_fault.md:313-317`), matched by
   `kfifoConvertInstToKernelChannel`'s linear scan over live channels
   (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:573-656`), with
   `INST_LO`/`INST_HI` occupying dwords 0–1 of the 32-byte packet
   (`simulated_gpu_fault.md:86`). `instanceMem.base` **is** that key, and it is the one field of the
   record we currently cannot source. ⇒ One of four stated blockers removed. `[src]`

2. ★★ **It answers `guest_memory_lock.md` §3.3 item 2 from the wire instead of from a bench
   experiment.** That row is a *candidate* pending an *"OPEN QUESTION (bench experiment): which
   aperture carries instance-block writes"* (`docs/design/guest_memory_lock.md:342-347`).
   ⊘ It does not need a boot: `instanceMem.addressSpace` **declares** the aperture in the alloc
   params, per channel, and `GL3` makes the wrong answer a refusal. A decoder settles it.

3. **A canary, explicitly not an oracle.** Assert `ib(c) == 0` for every channel. ⚠ Its green means
   nothing and must be documented as meaning nothing; ★ its **red** would refute §3.1 — something
   other than a real GSP wrote a channel's `PAGE_DIR_BASE`, and our model of the machine is wrong.
   Cheap, and the correct direction for a check whose informative outcome is failure.

### 5.3 ★★ WHERE THE SECOND DERIVATION ACTUALLY IS

The brief's real requirement — *route 4 is a single derivation and needs a check* — stands. Two
candidates survive this investigation, and neither is the instance block:

- ★★★ **The observed CE page-table write, attributed by destination-FB-address → owning PDB and
  latched at the CE release semaphore.** This is populate source **(2)** in `mode2_address_table.md`,
  co-equal with bind-time bindings, and it is genuinely independent: it is *the guest's engine writing
  page-table entries*, observed as data, not any control we also route on. `[src]`
  ⚠ It is independent of the publication in **origin** but not in **use** — confirm before relying on
  it that the attribution step does not resolve through the same published root, or it collapses into
  the tautology this document refused.
- ★★★ **`NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` (`0x801813`) — and this is the best of the three.**
  The guest states `physAddress` (the root), `flags.APERTURE`, `numEntries` and `hVASpace`
  **outright**, with `hVASpace == 0` meaning *"the implicit VA space associated with the
  client/device pair"* (`ogkm-580: ctrl0080dma.h:792-815`) — which is precisely the Device-default VA
  space route 4 resolves by name. ⇒ A **second, guest-stated, transport-independent** value for the
  same root, arriving on a different control from the `0x90f10106` publication. The C captured both
  and recorded that they **can disagree for one `hVASpace`** — *"`0x3114000` from `RESERVED_PDES` vs
  `0x3400000` from `SET_PAGE_DIRECTORY`"* (`C: nvkvm_gpu_emul.c:2754-2756`), which is what a real
  oracle looks like: capable of returning a mismatch. ★ Live work as of `3686b8b` (§16.29.5b).
  ⚠ Before treating a disagreement as a defect, read the C's own handling: it **appends** the
  `SET_PAGE_DIRECTORY` root as an extra candidate rather than overwriting, because the two describe
  the VAS at different moments — so the predicate is about a *rebind ordering*, not a plain equality.
  ⚠⚠ **And ONE PRECONDITION IS UNMEASURED, which decides whether this is an oracle for route 4 at
  all**: whether the `0x801813` seen on this boot carries `hVASpace == 0`. ⊘ Nothing yet shows it
  does. §16.29.5b infers *"route 4's own object"* from the header's `hVASpace == 0` semantics, while
  §16.29.5 — one section earlier, same rung — records that **who issues the SET is not settled**. And
  the C's record cuts the other way: `0x801813` is UVM's transport for **user** VASes, and
  *"Kernel-internal VASes (CeUtils scrubber etc.) never take this path"*
  (`C: nvkvm_gpu_emul.c:375-379`); the one divergence the C measured was on `hVASpace 0xcaf00005`, a
  UVM **dup handle**, not a Device default. ⇒ **Read `hVASpace` off the wire before promoting this to
  a second derivation.** If it is non-zero, this control names a different VA space and corroborates
  nothing about route 4. ★ `read_the_caller_not_the_id`, applied to a field instead of a caller.
- **A GPU fault's `INST_LO`/`INST_HI`**, once §5.2 item 1 lands: hardware naming the faulting channel
  by its instance block. ⊘ Blocked behind everything in `simulated_gpu_fault.md` §5.2, and it reports
  a channel identity rather than a root — a check on *attribution*, not on the PDB.

⚠ **And a third option that is not a derivation but is honest: leave route 4 single and say so.**
`execution_plane_increments.md` §16.28.12 already records that route 4 **mints nothing** — both the
name and the address are the guest's own — and that the walk **succeeded** on real addresses
(`va=0x12006c004 -> S:0x4d09004`). A single derivation from the guest's own publication, labelled as
single, is a weaker claim than a corroborated one and a much stronger one than a corroboration by a
constant.

---

## 6. Residue

1. **Nothing here was booted.** Every claim in §3 is `[src]` at `ogkm-580: 580.159.04` or `[measured]`
   as a grep with its exit code recorded; the C's `[measured]` results are quoted from its committed
   record, not re-run. ⊘ No bench time was taken and none is owed — the argument is from the driver's
   source, which is the right instrument for *"who writes this field"*.
2. **The 610 tree was checked first and its citations were discarded.** `research_clones/ogkm` is
   `610.43.02`; the bench driver is `580.159.04` (`ogkm_is_versioned`). Every line number in §3 was
   re-verified against `research_clones/ogkm-580.159.04` and several moved (e.g. the RPC fill site is
   `:2628` on 610 and `:2730` on 580). ⚠ Any future reader adding to §3 must re-verify against the
   tree the bench actually runs.
3. **§5.2's parse is unimplemented.** This document argues it is worth ~40 lines and a table row; it
   does not land it. The `ChannelInstMemWire` shape in §1.3 is `[inferred]` from the two existing
   precedents, not compiled.
4. **§5.3's first candidate is unaudited for independence.** Named as the thing to check next, not as
   a resolved second derivation.

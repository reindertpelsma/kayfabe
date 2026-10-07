# USERD IS NOT THE RING — adjudicating guest-USERD passthrough

> ### ⊘⊘ CORRECTION FOLDED IN, 2026-08-12 — §3 IS SUPERSEDED, §1 AND §4 STAND
>
> **§3 ("USERD IS NAMED PHYSICALLY, THE RING IS NAMED VIRTUALLY") is right about the naming and
> wrong about the consequence.** It concludes USERD needs *"a **different** crossing … byte
> identity between the guest's BAR1 view and a host-describable page"*, and that this is
> "exactly and only R32's J1/J2". §2's *"there is no host page to hand RM"* is likewise
> **overtaken**: the framebuffer memfd backing landed (`join_fb_leaf` /
> `BackingBytes::JoinsGuestWindow`) and R32 **ran** at `nvkvm-rs@f58473f`, with J2 holding.
>
> ★★★ And the missing address was never missing. The guest's **own CPU-RM** resolves
> `hUserdMemory[0]`/`userdOffset[0]` to a physical address before the GSP RPC — GSP has no
> client handle namespace, which is §1's own GSP row read forwards instead of backwards — and
> sends it as `NV_CHANNEL_ALLOC_PARAMS.userdMem` @ **+168 (580)**
> (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2747-2757`; the sub-memdesc at
> `kernel_channel_gv100.c:234-237` means `userdOffset` is already folded in). ⇒ **§5's
> dependency table collapses to items #3, #5, #7 and #8**; #1 and #2 landed and #4/#6 change
> shape. Full argument: `nvkvm-rs/docs/design/userd_mem_is_on_the_wire.md`.
>
> ⊘ **What stands, unchanged and load-bearing:** §1 (RM permits it — no aperture, class or
> allocation gate; 512 B / 512 B-aligned / non-VPR / < 2^40), §0.2 (`mem_phys` is irrelevant —
> and this was the half the successor doc got right), §0.3, and **§4, which is now the live
> cost**: a passthrough USERD cannot be CPU-mapped, so `GP_GET` stops being readable by us.
> §6's demotion of G8 stands and hardens.

**STATUS: LIVE, 2026-08-11, ⊘ §2/§3 SUPERSEDED 2026-08-12 — see the block above.** Read-only investigation, no code written, no bench run. Answers
the question *"can the channel's USERD be the guest's own page rather than one we allocate?"*
against the owner's passthrough item 5 (*"no ring/pushbuffer/semaphore/**userd** we inspect for
prod code"*). Scope: `kayfabe` @ `carry-the-guests-engine-and-close-the-ring-gate` (`11cced9`),
driver `ogkm-580.159.04`. Supersedes nothing. If the framebuffer acquires a shareable host
backing, §4 is the thing to re-open.

---

## 0. ★★★ WHAT I REFUTED FIRST, INCLUDING THREE CLAIMS OF THE BRIEF THAT COMMISSIONED THIS

### 0.1 ⊘⊘ "The guest's USERD alloc arrives with `size=0 params=-`" — MISATTRIBUTED, and it inverts the finding

The brief read `size=0 params=-` as *"the params are not decoded"*. Both halves are wrong.

- **The rows are C-era.** They live in `docs/reference/bench_evidence/run_gt1432_20e319b_probe.log`
  lines 66, 71, 74, 88, 96, 97. **No emitter for that format exists** in either tree today
  (swept `crates/`, and the C's `src/`/`tools/`/`scripts/`).
- **None of the six is a channel alloc.** Their classes are `0x0`, `0xcb33`, `0x80`, `0x2080`,
  `0x2081`. The log says nothing whatever about channel params.
- ★★★ **The guest's `hUserdMemory[0]` is decoded TODAY, version-keyed, on every guest channel
  alloc.** `ChannelUserdWire` — `crates/kayfabe-abi/src/notifier.rs:305-335`, `V580 {
  h_userd_memory: 32, userd_offset: 64 }`, `V610 { 36, 72 }` — dispatched by
  `DriverAbiTable::decode_channel_userd` (`crates/kayfabe-abi/src/versions.rs:1064`) and called
  at `crates/kayfabe-rmrpc/src/lib.rs:1382`, outside any `cfg(test)`.
  **Measured live:** `userd=h0xcaf00010/off0x0` —
  `traces/guest_boots/run_s19_1dfde1b_cup2_qemu.log:207`.

⇒ The brief's hypothesis — *"the same may be true here as for `engineType`; it may need a
version-keyed decode"* — is **correct in shape and already discharged**. `ChannelUserdWire`
needs **72 bytes** (580) where the `engineType` decode that shipped in `9d748b8` needs **132**;
the USERD wire is the *shallower* read and it landed first. What `9d748b8` added and this did
not is the **other half**: a consumer. `lib.rs:1379` says so in its own words — *"⊘ Read by no
decision"*.

### 0.2 ⊘⊘ `AllocFacts::mem_phys` having no producer is CONFIRMED — and IRRELEVANT to this question

The brief treats *"`RmGraph::backing_of` always returns `None`"* as the obstacle. The claim
verifies (declaration `crates/kayfabe-core/src/rmgraph.rs:383`; sole read `:2545`; every
`mem_phys: Some(..)` in the tree is under `tests/`). It is not the obstacle, for two independent
reasons:

1. ★★★ **Resolving the guest's declared handle was never the mechanism.** RM looks
   `hUserdMemory[0]` up **in the caller's own client** —
   `serverutilGetResourceRefWithType(hClient, hUserdMemory, classId(Memory), ...)`,
   `ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_gv100.c:184-187`. The
   guest's `0xcaf00010` is a handle in the **guest's** RM namespace and is meaningless to the
   host driver. Passthrough cannot be "forward the guest's handle"; it can only ever be
   "construct a **host** object over the **same pages**". `mem_phys` would not have helped.
2. **The physical page is already resolved, by a different instrument.** The guest reaches its
   USERD through **BAR1**, which our device GMMU-translates from a page directory *we* publish:
   *"BAR1 (translated): 0 reads / 483 writes resolved through the GMMU, 0 REFUSED by name;
   `bar1PdeBase = 0x2f1cac000` (the framebuffer address WE published…)"* —
   `traces/guest_boots/run_w254_e2b6c86_cel_hostdmesg_qemu.log:502` region. Every GP_PUT store
   already has a framebuffer physical address attached at the moment it lands.

### 0.3 ⊘ The comment that reads as a ruling and is not one

`crates/kayfabe-isolate-host/src/rm.rs:510-516` states:

> *"⊘ Always present, on both kinds of channel, and the asymmetry with the ring is the **design
> rather than an oversight**: USERD is **ours** on every channel we allocate…"*

★ **That is a description of the current state promoted to the grammar of an adjudication.** It
was written for the ring rung (`guest_ring_adoption.md` §5.11), where the contrast being drawn
is `RingOwner::Ours` vs `HandedIn`; it asserts USERD's provenance is deliberate without ever
having asked whether the guest's USERD *could* be used. The allocation site itself —
`rm.rs:3869`, `alloc_device_local(RING_OBJECT_BYTES)`, and the field write `rm.rs:3999`
`h_userd_memory_0: userd` — carries **no comment at all**.

⇒ Answering the brief's item 3 (*ruling, limitation, or accident?*): **a limitation, never
adjudicated.** Not an accident — the ring rung needed *a* USERD and ours was the only one
available — but nothing in the tree records a decision that it must stay ours. Per CLAUDE.md's
doc-hygiene rule, that comment should be corrected **in place**, above the sentence it
qualifies, rather than beside it in this file.

---

## 1. WHAT RM PERMITS — measured from `ogkm-580.159.04`, and it is permissive

All paths under `research_clones/ogkm-580.159.04/`. Version pinned at `version.mk:1`.

| property | finding | citation |
|---|---|---|
| client-supplied USERD is a **first-class path** | `hUserdMemory[0] != 0` gates the whole client-USERD body; `== 0` ⇒ RM self-allocates from its pool | `kernel_channel_gv100.c:80`; `kernel_channel_gm107.c:299-309`, `:108-130` |
| what class is required | **`classId(Memory)`** — any `Memory` subclass, looked up in **the caller's client** | `kernel_channel_gv100.c:184-193` |
| **must it be RM-allocated?** | ★ **NO.** `OsDescMemory` is *explicitly special-cased* — `refAddDependant(pUserdMemoryRef, ...)` exists precisely so client-supplied pages can back USERD | `kernel_channel_gv100.c:251-253` |
| aperture | **no rejection.** Both are first-class: the info path branches `ADDR_FBMEM` / `ADDR_SYSMEM` and errors only on neither | `kernel_fifo_ctrl.c:390-401`; no `ADDR_*` gate in `kernel_channel_gv100.c:151-278` |
| forbidden | VPR / protected memory only | `kernel_channel_gv100.c:199-202` |
| size (GA10x) | **512 B** (`1 << NV_RAMUSERD_BASE_SHIFT`, shift 9) | `kernel_fifo_gm107.c:1552-1555`; `dev_ram.h(gm107):49-50`; HAL selection `g_kernel_fifo_nvoc.c:835-843` (`_GM107` for all non-Tegra) |
| alignment | **512 B**, and the check only fires on a **nonzero** `Alignment` smaller than that — a memdesc declaring `0` passes | `kernel_channel_gv100.c:257-262` |
| physical-address ceiling | ★ **< 2^40.** `userdAddrHi = HI32(addr)` masked to 8 bits; `SF_MASK` is a width-mask-at-zero | `kernel_channel_ga100.c:46-47`; `SF_MASK` at `g_gpu_access_nvoc.h:388`; fields `dev_ram.h(ga100):27-28` |
| `USERD_INDEX_*` flags | select the 512 B slot **within RM's own channel-RAM page** (8 slots/page). Set by RM on the GSP path to pin ChID placement; **not** a description of a client handle | `alloc_channel.h:184-205`; `kernel_channel.c:2793-2802` |
| GSP offload | handle array is copied into the RPC, but **CPU-side resolves handle → memdesc → physaddr first** and GSP acts on `userdMem.base/size/addressSpace` | `kernel_channel.c:2292-2298`, `:2305`, `:2675-2683`, `:2748-2757`, `:2819` |

★★★ **The GSP row is the one that matters most and it cuts *for* passthrough.** The host driver
performs USERD resolution locally and ships GSP a physical descriptor. Nothing about a client's
handle namespace has to survive the RPC — which is exactly why an `OS_DESCRIPTOR` over pages we
chose is a legal USERD.

⇒ **Item 1 answer: RM permits guest-supplied USERD, unambiguously, and there is no aperture,
class, or RM-allocation constraint standing in the way.** The only hard numbers are 512 B,
512 B-aligned, non-VPR, below 1 TiB physical.

---

## 2. ★★★ THE BLOCKER IS NOT RM — THE GUEST'S USERD PAGE IS NOT HOST MEMORY

The guest's USERD is in the **emulated framebuffer**, and the emulated framebuffer is a
`HashMap` inside the QEMU process.

**Measured, `traces/guest_boots/run_w254_e2b6c86_cel_hostdmesg_qemu.log:502-520`** — the
guest's channel bring-up in fifteen BAR1 writes:

```
BAR1[0]  off=0x90000  val=0x20000000   GPFIFO entry lo    ┐ channel 0: ring @ 0x90000
BAR1[1]  off=0x90004  val=0x2801       GPFIFO entry hi    │
BAR1[2]  off=0xa008c  val=0x1          GP_PUT = 1         ┘          USERD @ 0xa0000
BAR1[3]  off=0xb0000  val=0x20100000                      ┐ channel 1: ring @ 0xb0000
BAR1[5]  off=0xc008c  val=0x1          GP_PUT = 1         ┘          USERD @ 0xc0000
BAR1[8]  off=0xe008c  val=0x1                               channel 2: USERD @ 0xe0000
BAR1[11] off=0x10008c val=0x1                               channel 3: USERD @ 0x100000
BAR1[14] off=0xa008c  val=0x2          GP_PUT = 2         → channel 0 advances
```

`0x8c` = `NV_RAMUSERD_GP_PUT` (`dev_ram.h(ga100):39`, word 35 × 4). Stride is **128 KB per
channel**, USERD at ring + `0x10000`, each USERD at **slot 0 of its own page** — so on this
workload there is no 8-slot sharing to untangle.

Those writes land here:

| link in the chain | state | citation |
|---|---|---|
| BAR1 write → framebuffer store | `SparseFb`, a `HashMap<u64, page>` | `crates/kayfabe-device/src/fbwin.rs:566-651`; stamped `byBAR1` at `:711` via `plane.rs:3006` |
| does that store have an **fd**? | ⊘ **NO.** `git grep 'memfd\|SharedRam\|OwnedFd' -- crates/kayfabe-device/src/fbwin.rs` → **zero hits** | — |
| the rung that would give it one — **R32, the framebuffer memfd JOIN** | ⊘⊘ **PRE-REGISTERED AND NEVER RUN.** *"P1–P5, P7, P8: UNSCORED. The probe was never run on hardware."* | `docs/design/fb_memfd_join_prereg.md` §5 |
| does w228's FB crossing substitute? | ⊘ **NO — see §3** | `docs/design/fb_leaf_crossing.md` |

⇒ ★★★ **There is no host page to hand RM.** Not "the wrong page", not "an unresolvable page" —
**no page**. The bytes the guest wrote exist only as heap in the QEMU process, which RM cannot
pin, a copy engine cannot read, and `alloc_os_descriptor` cannot describe.

---

## 3. ★★★★ WHY THE SECOND CROSSING DOES NOT REACH — USERD IS NAMED PHYSICALLY, THE RING IS NAMED VIRTUALLY

This is the finding the doc is named for, and it is the reason the ring's solved crossing
cannot be reused.

`fb_leaf_crossing.md` (§5.9, `w228`) built *"one blank host vidmem object per vidmem leaf,
mapped **FIXED at the guest's VA**"*. It is a **VA-keyed** crossing, and it is **blank** — it
establishes an address, not a set of bytes.

USERD never goes through a VA. The runlist entry names it by **raw physical address**:

```
NV_RAMRL_ENTRY_CHAN_USERD_PTR_LO      (31+0*32):(8+0*32)     dev_ram.h(ga100):27
NV_RAMRL_ENTRY_CHAN_USERD_PTR_HI_HW    (7+1*32):(0+1*32)     dev_ram.h(ga100):28
```

and RM fills them from `memdescGetPhysAddr(pUserdMemDescForSubDev, AT_GPU, userdOffset)`
(`kernel_channel_gv100.c:204-206`). No GMMU, no page table, no VA.

⇒ **A VA-keyed crossing cannot back a physically-named object, and a blank object cannot carry
the guest's cursor.** The ring got a crossing that fits it (`gpFifoOffset` *is* a VA, and R31
arm A proved host RM will accept a `memfd` → `OS_DESCRIPTOR` ring at a dictated VA —
`guest_ring_adoption.md:34`). USERD needs a **different** crossing: byte-identity between the
guest's BAR1 view and a host-describable page. That is exactly and only R32's J1/J2, and J1/J2
have never been measured.

---

## 4. ★★★ THE CONSEQUENCE NOBODY COSTED — PASSTHROUGH USERD TAKES `GP_GET` AWAY FROM US

Measured, R31 **arm B** (`guest_ring_adoption.md:37`): a guest-backed `OS_DESCRIPTOR` object
**cannot be CPU-mapped** — `NV_ESC_RM_MAP_MEMORY` returned `NV_ERR_NOT_SUPPORTED` (`0x56`),
with the driver's own `NVRM: memMap_IMPL: CPU mapping not supported for addressSpace: 0x1` in
the host `dmesg`.

The driver reason: `src/nvidia/src/kernel/rmapi/mapping_cpu.c:169-190` refuses CPU mapping of a
`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` unless `MEMDESC_FLAGS_ALLOW_EXT_SYSMEM_USER_CPU_MAPPING` is
set (or `ADDR_FBMEM` under MODS).

This cuts **both** ways and both are load-bearing:

- ★ **It enforces the owner's item 5 for us.** *"No userd we inspect"* would stop being a
  discipline we maintain and become a boundary **the host driver holds**. That is a strictly
  stronger property than the one item 5 asks for.
- ⊘ **It removes our `GP_GET` read.** Today `userd_cursors` (`rm.rs:4143`) and `submit_entry`
  (`rm.rs:4373`, `userd_store_u32(chan, USERD_GP_PUT, put)`) work through the CPU mapping made
  at `rm.rs:4090-4092`. On a passthrough USERD that mapping does not exist. The `GP_PUT` write
  is *supposed* to disappear — that is the point — but `GP_GET` is hardware's completion
  signal and we lose our read of it.

★★★ **And the replacement is the property R32 was designed to measure and did not.** R32's
**J2** is *GPU-write → CPU-read through a described memfd* — read the memfd through the
shell's mapping `S`, never through the described mapping `I`
(`fb_memfd_join_prereg.md` §1, §2 step 9). That is precisely, byte for byte, the shape a
`GP_GET` read on a passthrough USERD requires. ⇒ **J2 is not a nice-to-have for this design;
it is the design's completion plane.** The pre-registration already knew this — it flags J2 as
*"the direction `cuCtxCreate` is stuck on"* — and it went unrun for unrelated reasons (the
rung was reframed mid-flight and the bench sync timed out).

---

## 5. THE VERDICT

> ### **YES — with one unmeasured prerequisite, and it is a MEMORY prerequisite, not a driver one.**

**YES on the driver, unconditionally (§1).** RM accepts a client-supplied USERD, accepts an
`OS_DESCRIPTOR` over pages we chose, imposes no aperture constraint, and resolves the handle
CPU-side before GSP ever sees it. There is no reading of `ogkm-580.159.04` on which the driver
is the obstacle.

**NO today on the memory (§2), and the gap is exactly one unrun measurement.** The guest's
USERD page is `SparseFb` heap with no fd. R32 — the rung that gives the framebuffer a
shareable backing and proves the two-mapping join in both directions — is committed,
pre-registered, and **never ran**.

⊘ **UNSETTLED-BY-SOURCE on nothing.** Every sub-question resolved; no ambiguity in the driver
had to be worked around. The one thing standing between here and YES is a boot.

### What would have to change, in dependency order

| # | change | where | status |
|---|---|---|---|
| 1 | give the emulated framebuffer a **shareable host backing** (memfd), so a USERD page has an fd | `crates/kayfabe-device/src/fbwin.rs:566-651` | ⊘ not built |
| 2 | **run R32** and score J1 (write through `S`, describe `I`, GPU reads `S`'s bytes) and J2 (GPU writes, CPU reads through `S`) | `docs/design/fb_memfd_join_prereg.md` | ⊘ **never run** — the whole verdict hangs here |
| 3 | add a **`UserdSource` discriminant** to `alloc_channel_in`, mirroring the existing `RingSource` (`rm.rs:681`), so `h_userd_memory_0` can be a handed-in host handle | `rm.rs:3869` (the `alloc_device_local`), `rm.rs:3999` (the field write) | ⊘ not built; no discriminant exists |
| 4 | route `AllocFacts::userd` → `hosting` → `alloc_channel`'s argument list — **the identical seam `9d748b8` flagged unfixed for the CE instance** | `rmgraph.rs:377` → `crates/kayfabe-isolate/src/lib.rs:490` | ⊘ decoded, carried, consumed by nothing |
| 5 | stop `submit_entry` writing `GP_PUT` on that arm — precedent already exists (`RingOwner::HandedIn` → `RING_NOT_OURS`, `rm.rs:4348-4350`) | `rm.rs:4373` | mechanical |
| 6 | find a **`GP_GET` source** that survives the CPU-map refusal — §4; this is R32's J2 and nothing else | `rm.rs:4143` | ⊘ blocked on #2 |
| 7 | assert the page is **512 B-aligned, non-VPR, physical < 2^40** | new | mechanical |
| 8 | correct `rm.rs:510-516` **in place** — it reads as a ruling and is not one (§0.3) | `rm.rs:510-516` | doc hygiene |

⚠ **#3 and #4 are cheap and are NOT the work.** The temptation this doc exists to head off is to
build the `UserdSource` discriminant first, because it is the visible, satisfying, compile-checked
half — and then have a channel that hands RM a handle over memory that does not exist. **#1 and
#2 are the rung.** Everything else is plumbing behind them.

---

## 6. DOES G8 — THE CURSOR BRIDGE — SURVIVE?

> ### **YES, it survives — but demoted, and its justification is now a DIFFERENT one.**

`guest_ring_adoption.md:184-187` defines G8: *"Nothing writes the guest's `GP_PUT` into the host
channel's USERD, so a channel built this way is accepted by RM, schedulable, and **fetches
nothing**."* Nothing named G8 exists in code today; the `G8` tokens in
`crates/kayfabe-crec/tests/gsp_boot_gates.rs:344` and `crates/kayfabe-gsp/src/fault.rs:7` are an
unrelated GSP-boot-gate namespace.

**Why it survives:**

1. **It is the only mechanism that works without a framebuffer memfd.** USERD passthrough is
   gated on §5 items #1–#2, both unbuilt and one never measured. G8 is gated on nothing — the
   cursor is already trapped (`BAR1[2] off=0xa008c val=0x1`), already resolved through our own
   BAR1 GMMU walk, and the destination is already CPU-mapped (`rm.rs:4090-4092`). **G8 is
   buildable this week; USERD passthrough is not.**
2. **Even if #1–#2 land, G8's residue may be needed for `GP_GET`.** §4: passthrough USERD costs
   us the CPU mapping we read completions through. If J2 fails, a shadow USERD we *can* read is
   the fallback, and that shadow needs its cursor bridged in the forward direction anyway.

**Why it is demoted:**

⊘ **G8 must stop being described as the design and start being described as the fallback.**
`guest_ring_adoption.md` §4 lists it under *"What is NOT built"* — as a gap to be filled. After
this adjudication it is a gap to be filled **on a path that item 5 explicitly excludes from prod
code**. A bridged cursor is a cursor we inspect, and the owner's item 5 says prod must not.

⇒ **The honest formulation for item 5, until R32 runs:**

> Four of five are clean by construction. **USERD is clean-by-driver-permission and
> dirty-by-memory-topology**: `ogkm-580` will accept the guest's own page as USERD with no
> aperture, class, or allocation constraint, and the *only* thing preventing it is that the
> emulated framebuffer has no shareable host backing. Item 5 therefore carries a **dated,
> discharge-able exception**, not a permanent one — and its discharge condition is a single
> named, already-written, never-run measurement.

★ That is a materially better position than "USERD is ours and that is the design". The
exception has an expiry date and the expiry date is R32.

---

## 7. The falsifiers for anyone re-opening this

- **If RM turns out to reject a sysmem `OS_DESCRIPTOR` as USERD on real GA106** — §1's aperture
  row is refuted and the verdict becomes NO. The check is one ladder arm: R31 arm A already
  proves the memfd → `OS_DESCRIPTOR` → *ring* half on this exact GPU; the USERD half is the same
  object handed to a different field. ⚠ Nobody has run it. §1 is **source-measured, not
  hardware-measured**, and this tree has been wrong about that distinction before.
- **If R32's J1 fails** — two mappings of one memfd are not one memory across RM — then #1/#2
  are unachievable in the intended shape and G8 is promoted back to the design.
- **If R32's J2 fails** — GPU-write → CPU-read does not work through a described memfd — then
  USERD passthrough is achievable but **completion-blind**, and needs a separate answer for
  `GP_GET` before it can be prod.
- **If the guest ever packs multiple channels into one USERD page** (the 8-slot
  `USERD_INDEX_VALUE` shape, `alloc_channel.h:184`) — the measured 128 KB stride here says it
  does not on `cup2`, but that is **one workload**, and the quantifier is the thing this tree
  keeps getting wrong.

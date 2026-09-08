# w392 — ⊘ POINT 2's TRANSPORT IS NOT UNDRAINED, IT IS **UNREACHED**

**STATUS: LIVE, 2026-09-08.** Measured on a real GA106 (RTX 3060, open 580.159.04, KVM
template box `50260029`), source revision `aa4ffb4d`, stamp gate passed.
Supersedes nothing; extends `w391_the_three_point_coverage_ruling.md` §2.

---

## §0 — THE HEADLINE

The owner's three-point coverage ruling makes point 2 — **`MEM_OP`/`MMU_TLB_INVALIDATE` on
the guest's emulated channels** — a precondition for scrapping the doorbell's VAS
publication. Before this rung, its status read as *"the decoder exists, the sink is never
drained; wire the drain."*

**That was wrong, and the correction changes the work.** Measured:

| fact | value | where |
|---|---|---|
| `MMU_TLB_INVALIDATE` decodes reaching `apply_pushbuffer` | **0** | `MEMOP-CENSUS seen=0` |
| pushbuffer parses that ran at all | **0** | `grep -c FWD-RING` |
| doorbells in the same boot | **480** (`GrCompute=125 Ce=355`) | by-engine census |
| `CUP3_VAL` | **43** | compute passes; no regression |

⇒ **`parse_pushbuffer` never executed.** The census could not have read anything else. The
transport is not merely undrained — **no ring is parsed at all on this workload**, so there
is no arm onto which a drain could be wired.

---

## §1 — WHY THE ZERO IS TRUSTWORTHY THIS TIME

`CLAUDE.md` already records the C measuring this transport at **ZERO** on the Mode-2
compute path. A second zero would normally be worth nothing: *"a census ZERO needs a
KNOWN-POSITIVE"*, and without one it cannot separate *"the guest never invalidates"* from
*"the counter was never wired"*.

So the census ships with its own known-positive (`tests/tests/memop_census.rs`):
a real `PushMethod::TlbInvalidate` driven through the real `parse_pushbuffer`, asserting
(1) the decode arm runs, (2) the PDB is **read out of the guest's method words** and not
echoed from the channel's own VAS, and (3) the census moves. Plus a **negative control**:
a CE-only ring must leave the census flat.

★ The negative control caught a defect **in the test, on its first run**: the census is a
process-global `AtomicU64` and libtest is multi-threaded, so an exact before/after delta
straddled a sibling test's invalidate — a *true* report of a *false* premise. The fix was
to make the delta attributable (a file-local lock), **not** to weaken the assertion to
`>=`, which would have deleted the only check that catches a counter wired to every method.

⇒ The bench zero is a fact about the **guest and the transport**, not about the instrument.

---

## §2 — THE MECHANISM, NAMED EXACTLY

`kayfabe_rt::device::ring_content_is_forwardable` (`device.rs:6394`) is a **conjunction**:

```rust
matches!(route_of_engine(engine), DoorbellRoute::CpuCe)
    && matches!(kind, GuestChannelKind::Emulated)
```

and it is the sole gate above the sole call to `forward_ring`, which holds the sole call to
`parse_pushbuffer` on the live path (`device.rs:2907` → `:3144`).

`route_of_engine` maps `Ce → CpuCe`, `GrCompute|GrGraphics → HostGr`. So the parse runs
**only for a channel that is both CE-class and Emulated**.

### ⊘ And the boot could not say which half failed — the joint fact was never recorded

The log carried the two **marginals** on different lines — `engine=Ce` ×355,
`kind=emulated` ×202 — and the **joint nowhere**. `grep 'kind=' | grep 'engine='` returns
**zero lines**. Two readings fit and they have different fixes:

- **the CE doorbells were all Passthrough** (user procs) ⇒ the emulated channels that ring
  are GR, and UVM's `MEMOPS` pool never rang at all;
- **the emulated doorbells were all GR** ⇒ same conclusion by the other route;
- or a mix.

⇒ **This is why w392 adds one line at the gate** printing the pair *and naming the failing
conjunct* (`⊘ SKIPPED: ENGINE conjunct` / `⊘ SKIPPED: KIND conjunct`). A skip must arrive
as a reason, not as an absence. ⚠ Not yet read back from a boot at the time of writing.

---

## §3 — WHAT IS SETTLED, AND FROM WHERE

**`MEM_OP`/`MMU_TLB_INVALIDATE` IS a kernel channel and DOES land `Emulated` — by a chain
that is total, not heuristic.** The owner asked; this is the answer, every link cited in
ogkm 580.159.04 and in our own source:

| # | fact | citation |
|---|---|---|
| 1 | GA106's invalidate is `MEM_OP_D OPERATION=MMU_TLB_INVALIDATE`, class C56F | `kernel-open/nvidia-uvm/uvm_ampere_host.c:255` |
| 2 | every one rides `UVM_CHANNEL_TYPE_MEMOPS` on `gpu->channel_manager` | `uvm_mmu.c:62`, `:722` |
| 3 | that manager comes from `nvGpuOpsCreateSession` → `rmapiGetInterface(RMAPI_EXTERNAL_KERNEL)` | `src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:777` |
| 4 | ⇒ `privLevel >= RS_PRIV_LEVEL_KERNEL` ⇒ RM stamps `processID = KERNEL_PID` **in the RPC encoder, i.e. on our own wire** | `src/nvidia/src/kernel/vgpu/rpc.c:3382`, `inc/kernel/vgpu/rpc.h:69` |
| 5 | we decode `KERNEL_PID` → `ClientKind::Kernel` | `kayfabe-abi/src/guest_os.rs:286` |
| 6 | every declared Kernel client folds into the ONE system component at `SYSTEM_ANCHOR` | `kayfabe-core/src/project.rs:1026`, `:1170` |
| 7 | `channel_kind() → Emulated` ⇒ `trap_contract() → ScheduleAndReturn` | `kayfabe-core/src/project.rs:312` |

★ Link 4 is the load-bearing one and it is better than expected: the sentinel is stamped by
the **RPC encoder**, which is our transport. We do not have to infer privilege — RM tells
us, in-band.

### ⊘ The owner's worst case is a non-case, for two reasons

> *"lets say in the worst case it isn't [a kernel channel], and it does a CE copy to the
> address — it's interceptible only I think, its flakey as out of scope of what a CE copy
> should do."*

Correct, and there is a stronger reason. Under the passthrough data plane the host GPU
walks the **host** page tables, which host RM owns. A guest scribbling its own PTE pages
publishes **nothing to hardware**; it only *tells us its intent*. Missing one faults the
guest's own work. It is a liveness bug for that guest, never a breach.

---

## §4 — THE BLIND SPOT, STATED

`Ga10xPushbuffer::tlb_invalidate` (`kayfabe-chips/src/ga10x.rs:1457`) returns `None` for the
`TLB_INVALIDATE_PDB_ALL` form — *"`PDB_ALL` names no page directory; there is no `pdb` to
report"* — so a whole-GPU invalidate falls through to `PushMethod::Opaque` and is **counted
nowhere**.

That is right for a *publish* (there is no VAS to publish) and wrong for a *census*, so it
is written down rather than left to be rediscovered. `seen()` counts **PDB-targeted**
invalidates only. UVM's own emitter uses `TLB_INVALIDATE_PDB, ONE` (`uvm_ampere_host.c:258`),
so the path this census exists to measure is inside what it can see.

---

## §5 — WHAT THIS COSTS THE RULING

Nothing in the ruling changes. What changes is the **order of work**:

1. ~~wire the drain on `PushbufferOutcome::invalidates`~~ — premature; the arm never runs.
2. **read the `RING-GATE` line off a boot** and learn which conjunct fails. One boot.
3. depending on the answer: either UVM's `MEMOPS` channels never ring (→ the raw UVM-ioctl
   client the owner asked for becomes the known-positive that forces one), or they ring and
   are misclassified (→ a classification fix, much smaller).
4. only then wire the drain, onto an arm proved live.
5. only then scrap the doorbell's VAS publication.

⚠ **Step 5 stays last.** w390 measured that with publication off (`assert` arm) compute is
dead: `NO_KERNEL_LINE`, 16 × Xid 31. Scrapping publication before point 2 carries the
mapping loses compute, which is the owner's own sequencing concern.

---

## §5b — ★★★★★ THE UVM RAW CLIENT PASSES ON BARE METAL (2026-09-08, host of box 50260029)

`--uvm-invalidate`, real GA106, open 580.159.04, source `7311e312`:

```
ok W392C uuid          = GPU-b448b62a-2cac-58c2-46ff-4ecd1e67fc4b
ok W392C INITIALIZE    = rmStatus 0x0
ok W392C MM_INITIALIZE = rmStatus 0x0
ok W392C REGISTER_GPU  = rmStatus 0x0
ok W392C REGISTER_VAS  = rmStatus 0x0
W392C_OUTCOME=(P) THE CLIENT WORKS
```

★ **This refutes a sentence in our own source.** `blockage_coverage`'s doc says the
emulated-doorbell point *"needs a guest-KERNEL channel (UVM's), which a raw client cannot
allocate."* `REGISTER_GPU` returned `0x0` for a client that allocated **no channel at
all** — a raw client does not *allocate* the kernel channel, it makes **nvidia-uvm**
allocate one. The claim turned *"not built"* into *"cannot be done"*, and the one coverage
point with no raw-client arm was the one recorded as unreachable.

⊘ **The gate caught THREE defects in the client**, and the `NV_STATUS` *name* misdirected
two of the fixes: `0x5d` is `NV_ERR_PAGE_TABLE_NOT_AVAIL`, but `uvm.h:368` documents its
actual meaning for this ioctl as *"the UVM file descriptor [must] be associated with a
single process"*. The missing call was `UVM_MM_INITIALIZE`, not anything about page tables.
⇒ when a status code sends you at a mechanism twice and misses, read the header's **prose**
for that call, not the shared error-code table.

## §5c — ★★★★★ THE GUEST RUN: THE FALSIFIER SURVIVES, AND THE CENSUS IS STILL ZERO

**Owner's falsifier, 2026-09-08:** *"then it must pass on the guest if you have all maps."*

`[measured, boot w392cguest, Mode-2 guest on our emulated GPU, source ca78bd5e]`

```
ok W392C uuid          = GPU-78b352c7-1ccd-7a86-d282-49484c827f27
ok W392C INITIALIZE    = rmStatus 0x0
ok W392C MM_INITIALIZE = rmStatus 0x0
ok W392C REGISTER_GPU  = rmStatus 0x0
ok W392C REGISTER_VAS  = rmStatus 0x0
W392C_GUEST_OUTCOME=(P) THE FALSIFIER SURVIVES
```

★ **It passed.** The same client, byte-identical, that passes on bare metal completes the
entire UVM registration path against **our emulated GPU**. ⊘ And the UUID differs from the
host's (`GPU-b448b62a-…`), so this is our device answering, not a leak of the real one.

⇒ **On the registration path our coverage holds.** That is a real positive result and it is
the first time point 2's precondition has been shown to exist inside the guest at all.

### ⊘⊘ AND THE CENSUS IS STILL `seen=0` — BUT FOR A THIRD REASON, WHICH IS MINE

```
MEMOP-CENSUS seen=0 targeted=0
RING-GATE  — ZERO LINES IN THE WHOLE BOOT
```

`RING-GATE` printing **nothing** is the tell: `SharedDevice::doorbell` was never entered.
No doorbell rang, so nothing could have been parsed, so no `MEM_OP` could have been seen.

⊘ **The client registers and immediately unregisters. It never makes UVM do any work.**
`uvm_mmu.c:722`'s `tlb_invalidate_all` fires when the **page tree grows**, and a
registration that maps nothing grows nothing. So this zero is neither *"the transport is
missing"* nor *"the guest never invalidates"* — it is **the known-positive not yet being
one**, which is the same shape this file's §1 warns about applied to my own client.

⇒ **The next edit is small and named**: force a page-tree grow inside the client —
`UVM_ALLOC_SEMAPHORE_POOL` (`uvm_ioctl.h:957`, base 68) or an mmap on the UVM fd plus a
touch — and re-run. Only then does a `seen=0` say something about the transport.

## §6 — RESIDUE

- ⊘ The `RING-GATE` instrumentation is written and compiled but **its output has not been
  read from a boot**. Every §2 conclusion about *which* conjunct fails is therefore open.
- ⊘ Whether UVM's `MEMOPS` pool allocates channels at all on the cup3 workload is
  **unmeasured**. cup3 is one CUDA program; the LLM workload may differ.
- ⊘ The raw UVM-ioctl client (owner-requested) is **not yet written**. Its job is to force
  a UVM page-tree grow deterministically, so the known-positive does not depend on what a
  CUDA program happens to do.
- ⊘ **The client checks STATUSES, not CONTENT** — owner, same session: *"also needs to test
  for corruption of old mappings, and test that the contents is right."* A status-only
  client would have scored the w392 corrupted LLM run (16 tokens, garbage text, every
  `rmStatus` clean) as a pass. The extension is: write a pattern, churn other mappings,
  read the original back and compare. **Not built yet.**
- ◐ **The scale discriminator is CROSS-BOOT, not same-boot.** `MINMM_SUM=64` (w392b) and the
  16 garbage tokens (w392llm2) come from different boots, because w392b was cut before its
  LLM arm finished. The inference — *the defect scales with allocation count/size, not with
  the arithmetic path* — is sound but weaker than a single boot carrying both.

## §5d — ★★★★★ THE MEAN CLIENT IN THE GUEST: A DIFFERENT WALL, AND IT IS ONE LEVEL EARLIER

`[measured, boot w392dguest, Mode-2 guest, source b7fb876b]`

The client that passes **all five rows** on bare metal (`W392D_OUTCOME=(P)`,
`MEAN_FALSIFIER=PASS`) runs to completion in the guest and every row is **REFUSED** — with
one identical cause:

```
P1         → ⊘ REFUSED at engine read @P1 VA: the copy from 0x80_80000000 NEVER RETIRED
STALE RACE → ⊘ REFUSED at engine read @VA_X: the copy from 0x80_c0000000 NEVER RETIRED
P2         → ⊘ REFUSED at engine read @P2 VA: the copy from 0x90_80000000 NEVER RETIRED
P3         → ⊘ UNEXERCISED — P2 did not verify, and P3's readback IS P2's copy engine
```

★ **The client's design is what makes this readable.** It **refused** rather than reporting
a value, and P3 declined to run at all rather than report its reader's failure as the RPC
bind's. A status-only client would have said "all ioctls fine".

### ⊘⊘ AND THE DISCRIMINATION IS CLEAN — IT IS **NOT** A MAPPING FAULT

| signal | value | reading |
|---|---|---|
| host `Xid` during the run | **0** (0-line hostdmesg) | the host engine did **not** fault on any guest VA |
| `DOORBELL-VERB engine=Ce` | **12** | we **did** translate and forward every doorbell |
| `RING-GATE … kind=Passthrough` | **12** | every channel is Passthrough, so no ring is parsed — as designed |
| `MEMOP-CENSUS` | `seen=0` | consistent: nothing was parsed, so nothing could be counted |

Doorbells forwarded, **no fault**, **no completion**. Under the passthrough data plane the
host engine is supposed to read the guest's ring at identical VAs; a *missing* mapping would
fault and raise an Xid. **Zero Xids with zero completions means the host engine never
executed the work at all** — not that it executed against a bad translation.

⇒ **This is the recorded "the plane RINGS but does not COMPLETE" wall, reached by a raw
client for the first time.** It sits **one level before** every question the mean client was
built to ask: mappings cannot be graded in the guest until a submitted copy can retire.

⚠ It also means the guest arm of point 2 is blocked on a **different subsystem** — the
completion plane — and not on address-plane coverage.

### ★★★★★ §5e — ROOT CAUSE, IN OUR OWN DEVICE'S WORDS — AND IT IS THE SAME FACT AS POINT 2

```
adopt=DECLINED ⊘ an engine-object birth, so the armed path WAS consulted — and the
               address table held NO JOINED BINDING at this channel's ring VA
userd=DECLINED ⊘ the ring's leaf was consulted — and the guest's resolved USERD was
               UNREADABLE, in guest RAM, undeclared, or outside that leaf
               → RingSource::Ours(None)
[births=N guest_ring=0 guest_userd=0 declined=N not_asked=0 refused=0]
```

⊘ **`guest_ring=0`. Not one adoption in the whole boot.** (An earlier reading of "1" was
the *banner* line matching the grep, not an adoption — `a_count_cannot_see_a_substitution`,
again.)

**The complete causal chain, every link measured this boot:**

1. the guest maps its GPFIFO ring;
2. **we do not have that mapping joined in the address table** at the ring's VA;
3. at engine-object birth the armed adoption path is consulted, finds nothing, and
   **declines** → the host channel is born on `RingSource::Ours(None)` — *our* ring, empty;
4. the guest rings its doorbell; we translate and forward it (`DOORBELL-VERB engine=Ce` ×12);
5. the host engine reads **its own empty ring**, finds no work → **no completion, and no
   fault**, because nothing was ever translated badly;
6. every engine read in the mean client times out at "NEVER RETIRED", `Xid=0`.

⇒ ★★★★★ **THE COMPLETION WALL AND THE COVERAGE QUESTION ARE ONE FACT.** This is not a
missing completion architecture and it is not a missing semaphore writer — under the
owner's passthrough ruling we write neither. It is **a mapping we never learned about**,
one VA wide, and it is the ring's. Point 2 is not a side quest to the LLM corruption; the
ring VA is the first place its absence bites.

⚠ **The ordering is part of the requirement, not a detail.** The join must exist **before
the engine-object birth that would name it** — a publication that lands later cannot
retro-adopt a channel already born on `Ours(None)`.

### §5f — NEVER LEARNED, NOT REFUSED — and my secondary lead was WRONG

**Discrimination run (`traces/w392d_guest_wall/EVIDENCE.txt`, source `b7fb876b`).** Three verbs
name the ring VA `0x8000001000` this boot — `VAS-BIND-CENSUS`, `RING-PROJ`, `GUEST-RAM`.
**No join / bind / publish verb ever names it.** Refusals exist and are counted this boot
(22 `REFUSED`, `refused=1`×3, `refused=2`×7) — **none at the ring VA**.
⇒ the address table lacks a binding there because **nothing ever tried to make one**, not
because a predicate said no. A refusal would be my defect; an absence is a coverage hook.

⊘ **RETRACTED, same hour, by reading the code instead of two log lines.** I read
`PT-DECODE latched=0 requeued=817 rounds=0` under the w318 `⊘SKIPPED` banner as *"the dirty
gate is skipping a pass whose predecessor never completed"*. **It is not.** The gate sits on
**EXEC-WITNESS** (the *producer*), and its arming edge is stated in its own comment as
*"the store's EXECUTOR WRITE COUNT, not the page set"*, with `None` (unmeasured) **arming**
rather than skipping. No executor write ⇒ identical bytes ⇒ identical decode. `rounds=0` is
the **correct output of a sound skip**. ⚠ Two log lines are not a mechanism.

★★★ **And the retraction SHARPENED the finding.** `PT-DECODE`/`EXEC-WITNESS` decode
**executor-written FRAMEBUFFER pages**. The guest's ring is in **GUEST RAM** — the `GUEST-RAM`
verb is one of the three that names it. So our publication machinery does not miss the ring by
accident; **it operates on a different memory plane and structurally cannot reach it.** This
gap cannot be closed by extending PT-DECODE.

★★★★★ **AND THE EXACT SHAPE OF THE GAP, from `RING-PROJ`:**
```
ring=0x8000001000 entries=64 … GET=0 PUT=1 resY root=0x0/ap1/sh47 rootsrc=published
gp[0]@0x8000001000=0x8000000000+0x40  pb=V:0x10000  pbm[16w of 64B…]
```
**We READ the guest's ring completely** — descend, resolve (`resY`), see `GP_PUT=1`, decode
entry 0 and its pushbuffer. What we never do is **MAP it into the host GPU's VAS at that same
VA**. **Reading and mapping are different verbs, and only the first is built.** That is owner
ruling #231 verbatim (*"map the guest's ring/pushbuffer/USERD into the host GPU's VAS at
IDENTICAL VAs"*), re-derived from a raw client by a path that knew nothing about it.

### ⊘⊘⊘ §5g — CORRECTED SAME DAY, ON THE OWNER'S CHALLENGE. §5e AND §5f ARE BOTH WRONG.

The owner asked *"how do you mean the VA is never learned?"* — and the evidence bundle I had
already captured refutes me twice over.

```
VAS-BIND-CENSUS va=0x8000001000 vas=PRESENT rows=3
                hit=0x11000/Vidmem/start0x8000000000/len0x10000 … published=3
GUEST-RAM PIN   ring=0x8000001000 → NOT IN GUEST RAM (the table binds this VA in aperture
                Vidmem at 0x11000; `Binding::phys` is a guest-physical address ONLY for
                sysmem, so there is no file offset to ask the layout for.
                ⊘ Refused by name — nothing here reinterprets a framebuffer address as a GPA)
```

- ⊘ **"Never learned" is FALSE.** `RING-PROJ` resolves the ring, reads `GP_PUT`, decodes the
  pushbuffer. The VA is fully known.
- ⊘ **"The address table held no binding" is FALSE.** `vas=PRESENT rows=3 published=3`, hitting
  `Vidmem` at `0x11000`. The row exists. (The adopt path's *"no **joined** binding"* is about the
  **host-side** join, and is consistent with a present guest-side row.)
- ⊘⊘ **"Absence, not refusal" is FALSE, and it was the load-bearing half.** There **is** a
  refusal, **by name**: `GUEST-RAM PIN → NOT IN GUEST RAM`. It is a **correct** refusal — a
  framebuffer offset is not a GPA and nothing may reinterpret it as one.

★★★★★ **THE ACTUAL WALL: THE GUEST'S RING IS IN (EMULATED) VRAM, AND VIDMEM CANNOT BE EXPORTED.**
Identical-VA passthrough requires handing the host GPU the memory behind the ring. Our only
export mechanism (`GuestRamGrant` / OS_DESCRIPTOR) takes a **guest-physical address**, which
`Binding::phys` supplies **only for sysmem**. A vidmem-backed ring has no such address, so the
crossing does not exist today — for the ring or for anything else the guest places in VRAM.
⇒ This is **owner item #271** (*"MAP VIDMEM INTO THE GPA — the device-node mmap route we
specified and never issued"*, 2026-08-27), reached from a completely different direction.
⊘ It **retires** the RM-map-hook recommendation of §5f: hooking RM's map path would have
published the same Vidmem row we already have, and changed nothing.

⚠ **HOW A REFUSAL SCORED AS AN ABSENCE — and it is a class already in memory.** I grepped for
*join / bind / publish* verbs at the ring VA, found none, and concluded nothing had been
attempted. **The verb that attempted it is called `GUEST-RAM PIN`.** My vocabulary list did not
contain it, so its named refusal read as silence — exactly
`a_census_over_transports_is_as_complete_as_its_list`, which this campaign paid for at w326 and
which I re-ran today. ★ **A census over verbs is only as complete as its verb list, and the verb
you did not think to list is the one holding the answer.** The discrimination
*"refused vs never attempted"* is only sound if the refusal vocabulary is enumerated from the
**code**, not guessed from the log.

### ⊘⊘⊘ §5h — "VIDMEM CANNOT BE EXPORTED" IS FALSE. THIRD CORRECTION, AND THE POPULATION WAS WRONG TOO.

The owner asked *"a vidmem-backed ring cannot be exported at all today — why not?"*. It can. I
asserted an impossibility from **one path's honest refusal message**, without reading the code.

**1 — the vidmem crossing EXISTS and is built.** `SparseFb::joined` (`fbwin.rs:843`):
*"Framebuffer ranges served from memory a **second party also maps** … A joined range's pages are
`mmap`ed by the isolate too, so **the guest's write through this window and the engine's read
through the GPU MMU are the same byte.**"* Installed by `SparseFb::install_join`.

**2 — an APERTURE IS NOT A BACKING.** `kayfabe-fwd/src/lib.rs:3274`: the scratchpad carve-out
*"goes through `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over host pages, and it **does** present a
`Vidmem` aperture here, **because the region's aperture is the guest's declaration and not the
object's class**."* ⇒ `Aperture::Vidmem` means *the guest said vidmem*, **not** *unexportable*.
`GUEST-RAM PIN`'s refusal is correct **about the guest-RAM path only** — it needs a GPA/file
offset, which exists only for sysmem. I generalised one path's refusal into a universal law.

**3 — and it IS joined, in the line I quoted without reading:**
`RING-PROJ … GET=0 PUT=2 JOINED-one-memory … ring=0x8000001000` (line 127).

★★★★★ **4 — AND I HAD THE WRONG POPULATION OF CHANNELS ALL ALONG.** The births split in two,
and only one kind says what I reported:
| isolate | n | adopt | reason |
|---|---|---|---|
| `iso0` | 9 | `DECLINED` | *"the armed path WAS consulted — and the address table held no joined binding at this channel's ring VA"* |
| `iso2` | 7 | `NOT-ASKED` | *"a doorbell materialization: **this birth path offers no ring at all**, so nothing was consulted"* |

**The mean client is `proc=2`, its ring VAS is `0xcafe0004`, and
`grep -c "GR-BIRTH.*cafe0004"` = 0.** Every `iso0` birth is on `0xcafe0005/000c/0012/0019/001f`.
⇒ **The `adopt=DECLINED` lines I built three sections on belong to OTHER VASes.** The client's own
channel VAS never has a host channel born for it at all — it is not declined, it is **absent from
the birth population**.

⚠ **STOPPING HERE RATHER THAN NAMING A FOURTH ROOT CAUSE.** Open, and not to be guessed: why no
birth carries `0xcafe0004`, and whether `NOT-ASKED` (a birth path that offers no ring by
construction) is the client's actual path. Three successive readings — *"never learned"*,
*"absence not refusal"*, *"vidmem cannot be exported"* — were each produced by inferring a
mechanism from log prose and each was refuted by reading the source the log was printed from.
★ **The instrument that broke all three was the owner asking one short question.**

### ⊘⊘⊘ §5i — "EVERY CHANNEL IS PASSTHROUGH" IS FALSE. THE REAL CENSUS SAYS THE CLASSIFIER IS RIGHT.

The owner: *"many kernel channels should be Emulated."* Correct. From `channel_kind_census()`
(`device.rs:4070`), the census built for exactly this question:
```
CHANNEL-KIND AGREES emulated=2 passthrough=1 system_proc_channels=2 user_proc_channels=1
CHANNEL-KIND AGREES emulated=2 passthrough=5 system_proc_channels=2 user_proc_channels=5
CHANNEL-KIND AGREES emulated=6 passthrough=2 system_proc_channels=6 user_proc_channels=2
CHANNEL-KIND ⊘VACUOUS emulated=0 passthrough=0 … NO LIVE CHANNELS AT ALL … AT=teardown
```
⇒ **emulated channels exist (2–6 live), and the verdict is `AGREES`**: `system_not_emulated=0`,
`user_not_passthrough=0`. Every system channel is emulated, every user channel is passthrough.
**The classifier is correct and was never in question.**

⊘ **My "12 Passthrough / 0 Emulated" was NOT A CENSUS.** `kind=` is printed only by the
`RING-GATE` line I added at the **sole `forward_ring` call site**. Emulated channels have
`trap_contract() → ScheduleAndReturn` — **they are handled and returned and never reach
`forward_ring`.** The instrument could not see them *by construction*, and I read its silence as
their absence. ★ Note the shape: this is `every_row_verified_over_zero_rows` and
`a_census_over_transports_is_as_complete_as_its_list` **at the same time**, and the correct
census already existed in the same binary, rendered at `shim.rs:9247`.

### ⊘⊘ AND THE DEFINITIONS I MANGLED (owner, verbatim)
- **fake FB** = the emulated framebuffer = **ordinary host RAM** = the **emulated** plane. Rings
  there belong to emulated channels, and those are the rings we parse.
- **vidmem** = real GPU VRAM. Rings there are **ours**, for channels **we** manage (scratchpad).
- **passthrough** = the guest's own channels. Follow the guest: mmap into GPA with correct
  cacheability for real FB, DMA when it reaches sysmem. **Never parse.**
- ⊘ *"Passthrough channels are running against a fake framebuffer"* is a **category error** —
  fake FB IS the emulated plane's memory; it is not a thing a passthrough channel runs against.
  I also used `Aperture::Vidmem` to mean "real VRAM" and "the fake framebuffer" in adjacent
  paragraphs, when the code says it is the latter and that the aperture is the **guest's
  declaration**, not the backing.

★★★ **THE HYPOTHESIS THIS OPENS, AND IT POINTS AT MY OWN CLIENT.** `--uvm-mean` is a *user*
process, so its channels are passthrough — correct. But its GPFIFO ring resolves to
`Aperture::Vidmem`. Under the ruling above, **a guest ring in vidmem is a configuration we must
not support by construction.** If my raw client allocated its ring in vidmem, then it is testing
an unsupported shape and the defect is in **the client**, not in kayfabe — which would also
explain why real CUDA (`cup3`, `CUP3_VAL=43`) computes on this same build. ⚠ Hypothesis, unrun.

### ⊘⊘ §5j — OWNER SCOPES THE RULE: IT IS ABOUT PARSING, NOT PLACEMENT. §5i's HYPOTHESIS RETRACTED.

*"A guest ring in vidmem **we parse** is a shape we should not support by construction. Real guest
rings may live in vidmem (or sysmem if that's possible) but only in passthrough. For emulated
channels all rings live in real ram, regardless what the guest kernel says. I would not put it
this strict though — **any GPGA can be used as ring for emulated channels**, just it's preferred
that rings we parse is in fake fb and therefore in ram. Any GPGA is mappable anyways."*

⇒ The forbidden thing is **the verb**. Placement is free: passthrough rings go wherever the guest
puts them; emulated rings may use **any GPGA**, with fake-FB/RAM a *preference for parsed rings*.

⊘⊘ **§5i's closing hypothesis is RETRACTED.** I proposed the mean client was at fault for putting
its GPFIFO ring in vidmem. **It is not** — a user proc's channel is passthrough, and a vidmem ring
there is legitimate. I would have "fixed" correct code on the strength of a rule I had just
mis-stated.

★ **And the rule is already honoured on the functional path:** `ring_content_is_forwardable =
CpuCe && Emulated` gates the only live parse, and the census returns `AGREES` with
`system_not_emulated=0 user_not_passthrough=0`.
⚠ **One violator: `RING-PROJ`**, a read-only diagnostic that descends a passthrough ring and
decodes its pushbuffer. *"By construction"* means structurally unable, not harmless — and it
decodes untrusted guest content for no functional reason.

⊘ **Which leaves the guest-red mean client WITHOUT AN EXPLANATION.** Every candidate I raised
today is now withdrawn. State the absence rather than reach for a sixth.

### ★★★★★ §6 — THE OWNER'S RULE, AND IT INVERTS §5i/§5j's INSTINCT

*"If your raw client passes consistently on bare metal but not in the guest, then the only valid
explanation for being not a kayfabe bug is that your client did not hold the protocol contract
NVIDIA, was therefore based on heuristics, and CUDA would never execute like that. But that's a
rare case that needs good support, most often it's a kayfabe bug."*

⚠ **I reached for "the client is at fault" twice today and was wrong twice** (§5i's vidmem-ring
hypothesis; then the UVM-VAS reading below). The rule names the bias.

**Applied to the live finding, it INVERTS the conclusion.** `--uvm-mean` places its ring via
`UVM_MAP_EXTERNAL_ALLOCATION`, and the nvdiff oracle measures the guest in lockstep with hardware
**to that exact call — 221 of `cuCtxCreate`'s 479 ioctls.** A UVM-placed ring is inside NVIDIA's
contract; CUDA does precisely this. ⇒ the adoption path finding a `JoinsGuestWindow` binding for
RM-managed placements and **not** for UVM-managed ones is **a kayfabe gap**, not a client defect.
`RingSource::OursPlaced` names the condition itself: *"already placed at `ring_va` by
`nvidia-uvm`, in a VA space RM does not manage."*

**Measured contrast (same binary, two arms):**
| | `--ce-client` (w283) | `--uvm-mean` (w392d) |
|---|---|---|
| births | `guest_ring=1 declined=0` | **8 DECLINED, 7 NOT-ASKED, 0 GUEST-RING** |
| `joined=` on the birth line | `joined=YES` | **absent** |
| USERD vs ring leaf | misses by 1 byte ⇒ `userd=DECLINED` | inside the leaf — **passes** |
| ring placement | RM-managed VAS | **UVM-managed VAS** |

⇒ my client clears the USERD blocker that stopped `--ce-client`, and fails one rung earlier, at a
join that w283 shows working 88 times for the RM-placed case.

### ★★★★★ §7 — OWNER: AN UNPUBLISHED GPFIFO PAGE IS LEGAL. THE DEFECT IS THE FALLBACK.

*"GPFIFO can exist on a page that isn't published, for example continuing advancing when it got
unpublished, but I agree this is very rare, but not a violation, so it's debug only."*

⇒ **No invariant "the ring's page must be published."** A channel may legitimately keep advancing
over an unpublished page. Rare ⇒ **debug log**, never a refusal of the guest.

⊘ **§6 called the `adopted_guest_ring` refusal *"correct — it is protecting me."* Over-stated,
and it would have aimed the fix at enforcing an invariant that does not hold.** The accurate
split:
- Declining to adopt a `ShadowsGuestMemory` twin: **defensible** (its doc: a channel born over one
  *"decodes zeros, never advances `GP_GET`, and reports no error at all"*).
- ⊘ Falling back to **`RingSource::Ours(None)`** — *our own empty ring*: **the actual defect.** It
  converts a rare legal state into **silence with no diagnostic**, which is exactly w392d's
  measured symptom (`Xid 0`, no completion, all rows `NEVER RETIRED`).

★ **THE RULE: publish it, or refuse BY NAME. Never quietly substitute a different ring.** A
substitution is indistinguishable from success in every log line the path prints.

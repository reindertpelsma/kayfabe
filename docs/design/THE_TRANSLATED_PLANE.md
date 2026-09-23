# THE TRANSLATED PLANE — how a guest's work reaches the real GPU

**STATUS: LIVE.** Written 2026-09-21/22 (w824); **amended in place 2026-09-22 (w824b)** — a
synchrony audit after the owner's group-D correction (§10.1). Each amendment is marked `[w824b]`
and says measured / cited / inferred; §5, §7, §9, §10-B, the verdict's X5 and §12 are corrected
**above** the text they correct. Settles the design that closes `forwarded=0`.
Sources: owner rulings of 2026-09-21, `[fable w824]`'s investigation, and ogkm-610 read directly.
Supersedes nothing; **collects** `gpga_is_one_reserved_object.md` (LIVE 09-10),
`THE_CONSTRAINTS.md` §46 · §55 · §56, and `THE_V3_PLAN.md` P4/P7 into one statement.

⊘ Read this first, then those for detail. Where they disagree with this file, this file is later.

---

## §1 — The problem this solves, stated as the measurement

The thin guest passes **15 of 30** arms against a bare-metal **30/30**. Seven of the fifteen share
one signature:

```
guest_tokens=1   stranded=1   forwarded=0   refused=0   client rc=0
```

⊘ Note `refused=0`: **nothing was refused by name.** The work ran on our CPU, and every arm passed
its own rows — because once the bytes are in the right place, both executors write identical bytes
and the content ledger cannot tell which one ran.

★ And the cause is **not a bug**. `kayfabe-rt/src/device.rs:8978`:

```rust
EngineKind::Ce => DoorbellRoute::CpuCe,   // unconditional
```

CE work is routed to the CPU **by design**. `channel_kind()` is a two-way switch and `Translated`
is refused at birth (`kayfabe-fwd:4324`). ⇒ **30/30 requires building `Translated`, and no tree has
ever had it** — the C artifact's green was CE-on-CPU (`m2cefwd=0`, `m2hostsem=0`).

⚠ **`15` is the honest floor.** `[measured w797]` turning the CPU executor *off* took the suite from
**18 to 15**; all six arms that "regressed" had been passing because our CPU did the GPU's work. A
future jump in the pass count that is **not** accompanied by `forwarded>0` is that same lie.

---

## §2 — The model, in five sentences

1. **GPGA is ONE RM object.** One device-local allocation covers the guest's whole framebuffer,
   reserved at start; if it fails, the VM does not start.
2. **It is mapped into a VA space at the guest's own FB offsets** — the **identity window**. A
   guest framebuffer-physical address `p` is the GPU virtual address `GPGA_VA_BASE + p`.
3. **A physical-aperture operand is therefore arithmetic**: rewrite the address, flip the aperture
   bit to `_VIRTUAL`, submit on our own channel.
4. **A virtual-aperture operand needs a host VAS that mirrors the guest's VAs**, built from FIXED
   slice maps we issue when the guest tells us to.
5. **Completions are native** — real hardware writes the guest's semaphore, because the memory it
   writes *is* the guest's memory.

⇒ There is no join, no table of FB ranges, no shadow, no copy, and therefore **no race with a guest
that writes while we work** — the defect class that produced seven coexisting designs.

---

## §3 — ★★★ THE IDENTITY WINDOW IS NVIDIA'S OWN MECHANISM, AND IT HAS A VENDOR NAME

This is not a workaround invented here. RM does it internally, and calls it **`fbAliasVA`**.

`ogkm-610 src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:1055-1056` (and `:1086-1087` for dst):

```c
srcAddr = srcAddr + pChannel->fbAliasVA - pChannel->startFbOffset;
retVal |= DRF_DEF(B0B5, _LAUNCH_DMA, _SRC_TYPE, _VIRTUAL);
```

RM's own log line for the mapping is *"identity mapped to VAS"* (`mem_utils_gm107.c:505-516`).

⇒ **A physical-aperture CE operand is turned into a virtual one by adding a base and flipping a
bit — by the vendor, in the driver we must satisfy.** ★ That is as strong a provenance as §50
level 2 gets: we are not emulating a mechanism, we are using the one RM already uses.

---

## §4 — The two apertures, and why the split is clean

> `[owner, 2026-09-22]` *"there is no compute on phys aperatures, phys aperatures can only come
> from ogkm anyways."*

This is the sentence that closes the design, because it says **who produces each kind**.

| aperture | produced by | operands | our answer |
|---|---|---|---|
| **physical** | **ogkm itself** — RM's kernel work: the CeUtils scrub, UVM's page-table writes, UVM vidmem migrations | FB-physical addresses | ★ **arithmetic** — the `fbAliasVA` rewrite of §3 |
| **virtual** | **everything a guest program does** — user channels, compute, GPFIFO pushbuffer VAs, semaphores | GPU VAs in the guest's own space | ★ **a mirrored host VAS** (§5) |

⊘ **So the two halves do not compete; they do not even overlap.** Measured, not assumed:

- `clc7c0.h` — the **GR/compute** class — declares **`PHYS` zero times.**
- `clc7b5.h` — the **CE** class — declares it **20 times.**

⇒ Compute *cannot* present a physical operand, so the rewrite was never needed for it; and RM's
kernel CE work is *exactly* what the rewrite was written (by NVIDIA) to serve. ★ **Each half has
precisely one answer and neither answer is a fallback.**

⚠ And it explains a thing that looked like a limit and is not: a GR kernel's pointers live in its
**parameters**, not in its methods, so nothing could rewrite them — which matters only if you
expected the rewrite to serve compute. Nothing does.

---

## §5 — Virtual operands: walk at the guest's invalidate

The residual, and the only part that is genuinely new work.

⊘ §56 rule 2 says **we store no VA tables** — no mirror of the guest's page tables. §56 rule 3 says
**we do not auto-map from observed MMIO.** So a virtual operand cannot be resolved from a table we
keep, and we may not infer a mapping from watching the guest write one.

★ **But the guest tells us, and it tells us at exactly the right moment.** A correct guest writes
its page tables and *then* issues an invalidate (`ogkm-580 uvm_mmu.c:800-809`: writes,
`wfi_membar`, `tlb_invalidate_all`). That invalidate reaches us as either:

- BAR0 **`MMU_INVALIDATE`** naming a PDB (the RM path; `ALL_VA` is hard-coded, so it is coarse), or
- **`MEM_OP_D MMU_TLB_INVALIDATE[_TARGETED]`** in a UVM pushbuffer (`clc56f.h:132-176`, carrying
  PDB + aperture + VA + size — far more precise).

### `[w824b]` ⊘ WHERE THIS RUNS, AND WHAT THE GUEST SEES WHILE IT RUNS — the text below did not say, and that silence is the synchrony mistake §10.1 names, one section earlier

The three steps below read as a sequence executed *at* the invalidate. They are not, and cannot
be: the BAR0 `MMU_INVALIDATE` is a **vCPU MMIO write**, and §41/§48 forbid anything in that trap
beyond an enqueue and a wake. So the shape is:

| who | does | blocks on |
|---|---|---|
| the **vCPU** in the write trap | records `(pdb, aperture, depth)` in a preallocated slot, bumps the wake word, **returns** | nothing (§48: µs, tail) |
| the **guest** | spins reading `TRIGGER` until it clears — `kgmmuCheckPendingInvalidates_TU102`, a bare `osSpinLoop`, **4 s graphics / 30 s compute** | ✔ its own decision (§48.1); served from a **page**, so it is not even a read trap (§53) |
| a **worker** | launches the walk kernel; on its completion issues the bounded map ioctls; **then clears `TRIGGER` in the served page** — that clear is the observability edge §49.1 protects | ⊘ **never a CUDA sync on its stack.** `[inferred]` the walk's completion reaches the loop as an fd: `cuLaunchHostFunc` on the walk stream writing an eventfd, so the walk is one more epoll entry like any CE completion |

⇒ The **4 s budget bounds the worker's tail-to-clear**, not the trap, and on overrun the guest
proceeds with stale TLBs *and a skipped `sysmembar`*, reporting nothing
(`rm_cannot_express_a_narrow_invalidate`). That is why §10's *"4 s budget"* sentence is a latency
bound and **not** a cost to be optimised: the structural question is the same as §10.1's — does
the walk go **through the loop**, or does some step wait on its own stack.

⚠ **The `MEM_OP` split has the same shape and the old tree never built it.** A UVM pushbuffer is
*CE page-table writes* → `MEM_OP` invalidate → (a release the guest waits on). The walk must read
tables the CE has **finished** writing, so the Translated worker submits the entries before the
`MEM_OP` with a release semaphore + an armed event, **returns to the loop**, and on that fd's
readiness walks, maps, and resumes the channel from the `MEM_OP`. ⇒ **A Translated channel is a
resumable state machine with a suspension point per `MEM_OP`.** `planreactor.rs` §3 named this
exact requirement — *"interleaving two chains on one thread needs the chain to be resumable — a
state machine over every `VerbPlan` variant"* — and the old tree answered it with a single
blocking lane. ⊘ A `wait_for_completion()` call anywhere on the Translated worker's stack is the
defect, however short it measures.

⚠ **And one deadlock to check rather than assume away** `[inferred, unmeasured]`: the walk is a
CUDA kernel on the **same GPU** the guest's channels run on. It cannot wait on the guest (compute
preemption time-slices it in), but the *converse* must be verified on hardware: a guest channel
that has stalled on a fault we have not yet mapped must not hold a resource the walker's context
needs. If it can, the walker must run on a channel the guest's work cannot block.

⇒ **At that point, and only then:**

1. Walk the guest's tables **in place in GPGA** — the PTX walker, **72/72 on hardware**, reading
   through the identity window. ⊘ No copy, no promotion, no dirty tracking: the tables are already
   in memory a GPU kernel can read.
2. Issue host **FIXED slice maps** at the guest's own VAs, so host RM builds page tables whose
   layout mirrors the guest's.
3. Keep a ledger of **our own** map handles — RM's `UNMAP` requires the handle back.

★ **This is rule-3-compliant because the invalidate IS the guest telling us.** We are not watching
writes; we are serving a synchronisation point the guest issues on purpose.

⊘ **The window is safe by construction, not by a lock:** a correct guest does not edit a space's
tables while invalidating them, and a guest that violates it corrupts **only itself** — no
breakout, which is the correct bar.

⚠ **The one open ruling.** Rule 2 plainly forbids mirroring the *guest's* tables. Whether it also
forbids the ledger of **our own** handles is **unstated**. If it forbids both, the alternative is
unmap-all/map-all per invalidate — **~1800 RM calls** — and that must be *measured* before it is
chosen. **This needs the owner.**

---

## §6 — Guest system memory: the second window

The same trick, one level out. UVM's sysmem-side operands are **guest DMA addresses**, not GPGA
offsets. ⇒ Map the whole guest memfd as **one `OS_DESCRIPTOR`** at `RAM_VA_BASE`, so a guest
physical address `g` is `RAM_VA_BASE + g`.

⊘ Unmeasured at whole-RAM size, and it pins all guest RAM. ⚠ Under a **vIOMMU** guest the operand
is an IOVA, not a GPA — **not arithmetic**, and it is a compatibility axis, not a bug.

---

## §7 — Completions

★ **Native for the write.** `SET_SEMAPHORE_A/B` carries a VA; with the mapping present before the
release, **hardware writes the guest's page** — sysmem through the RAM window, vidmem into the one
object. `Completion::for_route(Route::Translated, _) = Completion::Nothing` already encodes this:
nothing is owed, because the GPU did it.

⊘ **Two things are ours, and they should be named now rather than discovered later:**

1. **The interrupt** — relayed through our client's OS event. Whether a release posts one is *"not
   yet established"* (`the_interrupt_arming_model`).
   `[w824b]` ⊘ **"Relayed" is where the synchronous frame hides a second time.** A relay that is a
   worker thread reading the fd and calling into QEMU's interrupt controller under the BQL is the
   old shape wearing a new name. The hardware shape is **fd → KVM irqfd → guest MSI, with no
   thread in between** — the same eventfd that is an epoll entry for our loop is registered as
   the guest's interrupt source; the loop only *arms* (`the_interrupt_arming_model`: arm-directed,
   never broadcast). `[inferred]` — the mechanism is standard KVM; that a release on our channel
   posts the OS event at all is the unmeasured half, and it is the first thing step 3 should
   record beside `forwarded=`.
2. **`GP_GET`** — hardware writes the **host twin's** USERD, and a Translated channel's ring is
   ours. ⇒ The guest's `GP_GET` is **one word we author**, by construction.

---

## §8 — What is dead (§56), and what replaces it

| dead | replaced by |
|---|---|
| **joins** — any table of guest FB ranges → host objects | one object; an **offset** |
| **stored VA tables** — mirrors, address tables, VA→GPGA shadows | the walk at the guest's invalidate (§5) |
| **auto-mapping from observed MMIO** — pt-write latch, dirty gate, whole-VAS sweep, `witness_writes`, `ReachShadow` | the guest's **own** map/invalidate |
| **per-GPGA phys backings** — per-leaf allocation | backing exists because the object exists |
| **promote/demote of page tables** | walking them in place; ⊘ the premise was already broken — a **CE** page-table write into a promoted *host* page sets **no KVM dirty bit** |
| **the CPU executor** (§46) | a real engine; `may_cpu_move` bounds it to **8 bytes** meanwhile |

★★★ Together these delete the **seven coexisting translation designs** in `kayfabe-core/src/gpu.rs`
— all on the default path, each added when the previous did not work, none removed. ⊘ They were
never seven mechanisms to choose between: they are seven answers to a question this design
**stops asking**.

---

## §9 — Build order. Every gate is a measurement, never a unit test.

| # | build | gate |
|---|---|---|
| **1** | **Identity window** — reserve, one FIXED map of the whole object into a fresh VAS at a 1 GiB-aligned base above 1 TiB | `IDENTITY_WINDOW mib=<n> placed_as_asked=true map_ms=<n> copy_from_window=MAGIC`, magic written at an offset **> 4 GiB**. ⊘ Bare metal; **no KVM needed** |
| **2** | **Guest-RAM window** — whole memfd as one `OS_DESCRIPTOR` | pinned bytes = guest RAM; a CE copy vidmem→RAM lands (R17 shape) |
| **3** | **Translated for the scrub + kernel CE** (phys-only ⇒ pure §3 rewrite) | `forwarded=N` on tokens `0x00010001`/`0x00010004` (w801 measured **0**); `execute_ours_spans` calls **= 0** ⇒ §46 satisfied |
| **4** | **Walk-at-invalidate → host slice maps** (§5) | `MMUINVAL … named & missed` → **0** for store-resident roots; `[w824b]` **and the structural gate first**: `TRIGGER` read traps = **0** (served from a page), `worst_trap` on the invalidate write in **µs**, worker tail-to-clear printed as a tail, not a mean |
| **5** | **`route_of_engine`: user CE off `CpuCe`** | `stranded=0` on `--ce-client`, then group A |
| **6** | **UVM Translated channel** (the `MEM_OP` split) | `--uvm-mean` PASS **with `forwarded>0`**, and the UVM join path deleted; `[w824b]` the channel **suspends** at each `MEM_OP` and resumes on an fd — a blocking completion wait on the worker's stack fails this gate even when the arm passes |
| **7** | **GR on the mirrored VAS** | `cuCtxCreate → matmul bad=0 maxerr=0` |

⊘ **Order matters twice:** step 3 before step 6 (the scrub is phys-only, two tokens, and its gate
already prints); and **nothing is deleted before its replacement re-greens** — `uvm-mean`,
`uvm-invalidate` and `missing-page-fault` pass **today** on the path §56 deletes.

---

## §10 — Feasibility: the 15 non-passes, grouped by what unblocks them

| group | n | arms | unblocked by |
|---|---|---|---|
| **A** | 8 | blockage-coverage · late-map-race · executor-vas · dictated-ring · ce-client · cross-client-leak · engines · rpc-mixed-allocs | steps **1+4+5** — *one change* |
| **B** | 3 | defer-liveness · uvm-mean · map-stress | `[w824b]` ⊘ **not "merely slow" — the same falsifier as D at lower intensity.** The 3.8× is the old tree's off-trap path being **one blocking lane** (`planreactor.rs` §1: *"totally ordered … while it is parked in one isolate's `read`, no other isolate's work can even be started"*) plus a 7.34 ms synchronous sweep per invalidate. Under v3 these run at bare speed **or** they report the structural defect D reports. A budget ratio is the fallback only if the loop is proven and the residue is the walk itself |
| **C** | 1 | ce-client-guest-ram | step **2** — 13 000 per-row pins become slices of one window |
| **D** | 2 | concurrency · concurrent-fuzz | ★ **the acceptance test for the async design** — see §10.1. Not a performance unknown |
| **E** | 1 | gpga-reserve-probe | budget ≥ 90 s **and** the BAR1 device-view path |

### §10.1 — ★★★ GROUP D IS NOT UNCHARACTERISED. IT IS THE FALSIFIER FOR THE WORKER/EPOLL DESIGN.

> `[owner, 2026-09-22]` *"we have workers and the epoll loop, it stress tests that design. if its
> proper then it should work. No nvidia ioctl can actually stall for longer, eventfds are used to
> signal completion if sempahore fallback kicks in what libcuda in guest userspace does (or for
> translated channels automatically, we don't have to specially handle it anymore), and then its
> just one of the entries in the epoll list. The entire epoll loop worker thats asynchronous is
> PRECISELY designed to exist for that in the first place. **Thats why I said to add these
> tests.**"*

⊘⊘⊘ **Two hypotheses were offered for group D and BOTH were wrong**, in the same way: each
proposed a *cost* to be found, when the arms exist to test a *design*.

- `[fable w824]` per-process isolate spawn, 1.7 s × N clients. ⊘ Refuted for `--concurrency`: it
  was rewritten **in-process**, and its own comment says so — *"The verb is called **DIRECTLY**
  now. The old `with_rm` closure existed to cross the isolate's IPC boundary; in-process there is
  no boundary to cross."* No spawn per thread.
- **Mine**: a super-linear sweep over live VA spaces. ⊘ Plausible arithmetic, wrong frame — it
  assumed the work is synchronous and asked how expensive it is.

★ **The arms were added on purpose, as the stress test for the asynchronous plane.** The shape of
the answer is therefore not *"how many ms per verb"* but **"did the work go through the loop at
all?"**

**Why it must pass if the design is right:**

1. **No NVIDIA ioctl stalls for long.** A verb is bounded; nothing in the RM path is a long block.
2. **Completion arrives on an eventfd** — either the semaphore fallback that guest-userspace
   libcuda drives, or, for **Translated** channels, automatically. ⊘ *"we don't have to specially
   handle it anymore"* — a Translated completion is native (§7), so it needs no bespoke path.
3. **Therefore a completion is just another entry in the epoll list**, and 4 threads × 200
   alloc/free pairs is 800 bounded verbs and 800 fd readinesses. That finishes.

⇒ **A hang at 300 s does not mean the work is slow. It means something BLOCKED where the design
says it should have been an epoll entry.** That is a *structural* defect and it is visible without
timing anything: find the synchronous wait.

⚠ **And this is why group D belongs to v3 rather than to the old tree.** The epoll machinery lives
in `kayfabe-shell/src/reactor.rs` and `kayfabe-isolate-host/src/planreactor.rs` — **both inside the
isolate plane v3 deletes.** v3's replacement is the wake word plus `VmmOps::signal_worker` /
`signal_drainer`, with the loop itself in **`kf-qemu`, currently 0 lines.** ⇒ Group D is not
outstanding *diagnosis*; it is the **acceptance test for a component not yet written**, and it
should be run against `kf-qemu` the day it exists — exactly the role the owner added it for.

★ **So the estimate does not need a 300 s diagnostic first.** It needs `kf-qemu`. If group D still
hangs once the worker/epoll plane is v3's, **the design is wrong and that is worth knowing early** —
which is the whole point of having written the test before the code.

**Verdict: no arm is unreachable by construction.** 30/30 is reachable once: **X1** the identity
window · **X2** walk-at-invalidate with the handle ledger · **X3** user CE off `CpuCe` · **X4** the
guest-RAM window · **X5** `kf-qemu`'s worker/epoll plane exists and group D passes through it (§10.1). `[w824b]` — was *"a diagnosed cause for group D"*, the cost frame §10.1 refutes.

**Shape, stated honestly in both directions.** The deletions remove **code and the tail** —
promote/demote, dirty tracking, seven designs — which is most of what the earlier 4–6 week estimate
was costing. They do **not** remove the core risk: **X2** is the mechanism those seven designs
circled, and its cost now sits inside the guest's own **4 s TLB-invalidate budget, where a slow
`TRIGGER` clear is silent corruption rather than an error.** What is new is that every piece is
already measured — walker 72/72, R17 CE, `map_store_slice`, the invalidate trigger — and that
**nothing sits between the walk and the map**: no gate, no latch, no shadow.

⇒ **15 → 23/30 in ~2 weeks** if step 1 is green in the first hour. **30/30 in 3–4 weeks**; group D no longer
carries a separate unknown — it passes when the async plane is right, or it reports that the plane
is wrong, and either outcome arrives with `kf-qemu` rather than after it. ⚠ *Significant improvement* inside 1–2 weeks is
credible for group A; **30/30 inside it is not**, and should not be promised.

---

## §11 — The first experiment: one arm, one box, under an hour

```sh
BENCH_DIR=/root/bare scripts/fastguest/bare_metal_suite.sh idwin 120 --identity-window
```

Reserve the object, **one FIXED `MapMemoryDma` of the whole of it**, CPU-write a magic through a
BAR1 slice at an offset **> 4 GiB**, then CE-copy `src = GPGA_VA_BASE + off` as `_VIRTUAL` to
sysmem and compare.

**One line decides it:**

```
IDENTITY_WINDOW mib=11904 placed_as_asked=true map_ms=<n> copy_from_window=MAGIC
```

- `placed_as_asked=false`, or `0x51` ⇒ **the whole construction needs slices instead**, and the
  estimate moves by weeks.
- `map_ms` in seconds ⇒ group B is in trouble.
- MAGIC landing from beyond 4 GiB ⇒ **`VA = base + fb_phys` is retired as a question for the whole
  object**, which is the single largest unknown in this design.

⊘ **What is measured versus assumed, so the estimate can be audited:** FIXED placement of *slices*
is measured (`placed_as_asked=true`, w228). A FIXED map of the **whole** object has **never been
attempted** — every path in this tree maps slices, and `DL_MAP_BYTES` is deliberately 64 KiB.
Largest reservation measured: **11 904 MiB** noncontiguous; **6144 MiB** contiguous.

⚠ And a trap worth stating once: `0x51` (`NV_ERR_NO_MEMORY`) on a FIXED map is **address
occupancy**, not *"the same object is already there"* (`gpu_vaspace.c:1372-1380`). The C artifact
treated it as success; that happened to be right for server-reserved ctx VAs and, adopted
generally, **silently aliases a different object.** Keep this port's `VA_ALREADY_MAPPED` refusal.

---

## §12 — What we will not do

- Treat `0x51` on FIXED as success.
- Put the identity window in a Translated-only VAS — UVM mixes apertures in one stream ⇒ it goes in
  **every** host VAS we create, above the guest's range, from the first commit.
- Load libcuda into QEMU for pushbuffer reads; use the R17 CE. (UVM's pushbuffer is **sysmem** by
  default anyway — `uvm_pushbuffer.c:98-117` — so it is a memfd read at RAM speed.)
- Re-enable the CPU executor to move the scoreboard.
- Start with the UVM channel; start with the **scrub** — phys-only, two tokens, gate already prints.
- Delete the UVM join path before its replacements re-green.
- `[w824b]` Read a group-D **or group-B** hang as a *cost* — it is a synchronous wait where the design says there is an epoll entry; find the wait. (Was: *"Estimate group D without the 300 s diagnostic"* — the cost frame.)
- `[w824b]` Put a `wait_for_completion()` — CUDA sync, semaphore poll, socket read — on any worker's stack "because it is short". The mean is short; §48's tail is the statistic, and it is the shape that produced group D.
- Keep `Store::carve`: under one object the **guest's** RM heap chooses offsets; we never carve the
  guest's object. `Store` collapses to `{token, len}` + a bounds-checked `slice()`.


---

## §13 — `[MEASURED w825]` THE FIRST EXPERIMENT RAN. Half the construction holds; half is refused.

**Box 52236011, GA106, driver 580.159.04, bare metal, no KVM.** Rev `84f6347e`.

### ✔ The reservation holds, and better than required

```
IDENTITY_WINDOW_LARGEST_RESERVABLE_MB=11904      (advertised 12288)
STORE-RESERVE ★ CONTIGUOUS and 1 GiB-ALIGNED, 11904 MiB — every slice's physical address is
  `base + offset` with `base ≡ 0`, so a FIXED map is congruent at EVERY page size and store
  slices need no small-page pin.
```

⇒ **GPGA as ONE object is measured.** 11 904 MiB, **contiguous**, **1 GiB-aligned** — so
congruence holds at every page size and §2's premise stands. `ce_still_works=true` throughout.

### ⊘ The whole-object VA mapping is REFUSED, in every configuration tried

```
IDENTITY_WINDOW_RM_CHOICE=REFUSED NoMemory                    ← RM picks the address: refused
IDENTITY_WINDOW_TRY asked=0x10000000000 REFUSED Other(19305)  ← 1 TiB
IDENTITY_WINDOW_TRY asked=0x4000000000  REFUSED Other(19305)  ← 256 GiB
IDENTITY_WINDOW_TRY asked=0x2000000000  REFUSED Other(19305)  ← 128 GiB
IDENTITY_WINDOW_TRY asked=0x1000000000  REFUSED Other(19305)  ← 64 GiB
IDENTITY_WINDOW_TRY asked=0x800000000   REFUSED Other(19305)  ← 32 GiB
IDENTITY_WINDOW_TRY asked=0x400000000   REFUSED Other(19305)  ← just above the object
IDENTITY_WINDOW mib=11904 placed_as_asked=false at=none map_ms=0 ce_still_works=true
```

`Other(19305)` is **`VA_ALREADY_MAPPED = 0x4B69`** — this port's name for RM's **`0x51`
(`NV_ERR_NO_MEMORY`) on a FIXED map**, which §11 says must **never** be read as success.

★★★ **The address is not the variable.** Six bases spanning 16 GiB → 1 TiB refuse identically,
and RM refuses its *own* choice with the same status. ⇒ It is not placement. **RM will not build
the mapping at this size**, and `map_ms=0` says it decides instantly — not a timeout, not a cost.

### ⊘ What this does NOT yet show, stated so the result is not over-read

⚠ **The VAS was allocated with `vaSize = 0`** — *"the default range"* (`rm.rs:7377`, and the
comment says so: *"Per-`Vas` separation is the property that matters, not the geometry"*). **An
11.6 GiB mapping may simply not fit the default range**, in which case this is a limit of **how we
asked**, not of what RM will do. `NvVaspaceAllocationParameters` carries `va_start_internal` and
`va_limit_internal` (`kayfabe-abi/src/bringup.rs:282-284`) and neither has ever been set here.

⇒ **The next experiment is one field, not a redesign:** allocate the VAS with an explicit range
covering the object and re-run the same ladder. Until then §10's estimate stands unchanged — this
is **not** a refutation of the identity window, and must not be reported as one.

### ★ And two instrument lessons, both mine

1. **v1 of this arm discarded the error** (`mapped.ok()`), so the first run reported the bare word
   `REFUSED` and cost a full round trip. The same defect I spent the day fixing in five shell
   scripts — **reporting failure without reporting why** — written fresh, by me, hours later.
2. **One guess is not an experiment.** v1 asked for a single base; the *ladder* is what showed the
   address is not the variable, and RM's own choice is what showed it is not our arithmetic. Both
   cost nothing and both were absent from v1.


### §13.1 — `[w825]` FOUR ITERATIONS, AND THE MAPPING QUESTION IS STILL OPEN. What is settled and what is not.

⊘ **Settled, and it is the valuable half:**

| | |
|---|---|
| GPGA as **ONE** object | ✔ **11 904 MiB**, measured |
| **contiguous** | ✔ |
| **1 GiB-aligned** | ✔ ⇒ congruent at every page size; store slices need no small-page pin |
| an ordinary CE copy in a default VAS | ✔ `ce_still_works=true` |

⊘ **NOT settled: whether the whole object can be mapped into one VA space.** Four runs, and
**every one of the four was limited by my instrument rather than by RM:**

1. v1 **discarded the error** — reported the bare word `REFUSED`.
2. v2 asked **one base**; the ladder later showed the address is not the variable.
3. v3 built an explicit-range VAS with `va_size`/`va_base` only — **`ce_still_works=false`**, i.e.
   a broken space whose refusals meant nothing.
4. v4 added `SHARED_MANAGEMENT` per `nv_gpu_ops.c:2632-2637` — and **`ce_still_works` is still
   false**, which is now *expected*: `SHARED_MANAGEMENT` means **the client manages the range**,
   so RM will not choose addresses in it. Both `rm_choice` and `prove_ce_copy` pass `None`, so
   both must refuse. ⇒ **The probe is asking a shared-managed space to behave like an RM-managed
   one.**

⇒ **The open question is now precise**, which is the one thing four runs did buy:

> Under `SHARED_MANAGEMENT`, **every** map must be FIXED and inside the declared range. So the
> probe must (a) declare a range that *contains* the base it then asks for — v4's range was
> `[64 GiB, 128 GiB)` while five of its six bases sat outside it — and (b) stop using `None`
> anywhere, including in the CE-copy control.

⚠ **Do not read any of §13 as evidence against the identity window.** It is evidence that this
tree has never built a shared-managed VA space before, and that I guessed at the contract four
times instead of reading it once. ★ The one thing that *did* work immediately was reading ogkm —
`nv_gpu_ops.c` named the required flag in a single grep after two runs had failed to find it.

⊘ **Cost, recorded honestly:** ~1 h of box time and four round trips, all instrument. The
reservation result would have been worth the box on its own; the mapping result is not yet worth
anything.

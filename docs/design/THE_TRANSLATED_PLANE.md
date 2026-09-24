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
| **3** | ⊘ ~~**phys-only ⇒ pure §3 rewrite**~~ — **FALSE, see §19.** Needs step 4 first | `forwarded=N` on tokens `0x00010001`/`0x00010004` (w801 measured **0**); `execute_ours_spans` calls **= 0** ⇒ §46 satisfied |
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


---

## §14 — `[MEASURED w825]` ✔ THE IDENTITY WINDOW WORKS. It takes TWO maps, at 1 ms each.

**Box 52236011, GA106, 580.159.04, bare metal.** Rev `ea60b84b`.

```
IDENTITY_WINDOW_MAPPABLE mib=11904 REFUSED NoMemory
IDENTITY_WINDOW_MAPPABLE mib=5952  OK map_ms=1   ⇐ LARGEST
```

⇒ **The limit was the SIZE, and it is not a wall — it is a divisor.** 5 952 MiB is exactly half
of the 11 904 MiB reservation, so the guest's whole framebuffer is covered by **two** whole-object
maps at **~1 ms each**.

### §14.1 — Why two maps is not a compromise

§2 requires only that **a guest FB-physical `p` is the GPU VA `base + p`**. ★ That holds for **N
contiguous maps that tile the object at fixed offsets** exactly as it holds for one — the
arithmetic is unchanged, and the guest cannot tell. **One map was the ideal, never the
requirement.**

⊘ The earlier `placed_as_asked=false` runs were therefore asking the wrong question: they gated on
*"can the whole object be mapped in a single call"*, which the design never needed.

### §14.2 — What this settles, and what it leaves

| | |
|---|---|
| GPGA as ONE **object** | ✔ 11 904 MiB, contiguous, 1 GiB-aligned |
| the identity **window** | ✔ **viable — 2 maps × ~1 ms** |
| `p → base + p` arithmetic | ✔ unaffected by tiling |
| §9 step 1's gate | ⇒ **restate as "the window TILES the object"**, not "one FIXED map" |

⚠ **Still open, and cheap to close on the next run:**
1. The exact ceiling is **between 5 952 and 11 904 MiB** — the bisect halves and stops at the
   first success, so it has not been narrowed. Worth one more pass: if the true limit is ~8 GiB
   the tiling is still 2, and if it is exactly 6 GiB there may be a round number behind it.
2. **5 952 MiB was mapped with `None`** — RM chose the address. A **FIXED** map at a base *we*
   choose, at that size, is not yet shown. That is the production shape and it is the next gate.
3. The ceiling is **above 4 GiB**, so it is *not* a 32-bit artefact. What it *is* remains unknown
   and should be named before it is designed around.

### §14.3 — ⊘ Five iterations, and what the detour actually cost

Runs 1–4 measured my instrument (error discarded · one base · a broken VAS · a shared-managed
space asked to behave like an RM-managed one). Run 5 asked correctly and the answer arrived in one
line. ★ **The thing that turned it was not another experiment — it was reading `nv_gpu_ops.c`**,
which named the required flag after two runs had failed to find it by trial.

⇒ **The lesson, stated as a rule rather than a regret:** when a probe refuses and the refusal is
*uniform across every input you varied*, stop varying inputs. A uniform refusal is a statement
about the **call**, not about the values — and the call is documented in the driver we are
required to satisfy.


---

## §15 — ✔✔✔ `[MEASURED w825]` **THE IDENTITY WINDOW IS ESTABLISHED.** The first experiment passes.

**Box 52236011, GA106, 580.159.04, bare metal, no KVM.** Rev `6bebfa0f`, `ARM_RC=0`.

```
IDENTITY_WINDOW_SIZED reservable_mib=11904 mappable_mib=11857 delta_mib=47
IDENTITY_FIXED_RM_CHOICE=0x120000000 (mappable size, default VAS)
IDENTITY_WINDOW_ESTABLISHED base=0x120000000 mib=11857
    ⇒ guest fb_phys p is GPU VA 0x120000000+p
RUNGCTL_identity_window=PASS
```

★★★ **The guest's whole framebuffer is ONE RM object, contiguous, 1 GiB-aligned, mapped WHOLE in
a single call, at a base we know.** §2's premise is measured end to end, and §4's arithmetic —
*a physical-aperture operand is `GPGA_VA_BASE + guest_fb_phys`* — is sound on hardware.

### §15.1 — ⊘⊘⊘ EIGHT RUNS CHASED A PROPERTY THE DESIGN NEVER ASKED FOR

Every run from v1 to v8 gated on a **FIXED map at an address of our choosing**, and every one
refused with `0x51`. The design says:

> §4: *a physical-aperture operand is `GPGA_VA_BASE + guest_fb_phys`*

★ **Nothing there requires `GPGA_VA_BASE` to be a *particular* value.** It must be **known** and
**constant** — not **dictated**. RM handing us `0x120000000` is a perfectly good
`GPGA_VA_BASE`. I read *"we choose the base"* into a line that says *"the base is the base"*.

⇒ **The rule this cost: before measuring whether something is possible, re-read why it is
needed.** Eight round trips established that RM will not honour an address I had no reason to
insist on.

### §15.2 — And it explains every earlier refusal at once

RM places the window at **4.5 GiB**. ⇒ The default VA space does not reach 8 GiB, let alone the
32 GiB / 256 GiB / 1 TiB bases the ladder kept asking for. **Every base was outside the space,
and `0x51` said so six times.** ⊘ I chose them to be *"clear of everything"* — clear of what host
RM self-reserves low, clear of the guest's own range — which put them clear of **the space
itself**.

### §15.3 — Sizing: derived from what is MAPPABLE, not what is reservable

| | |
|---|---|
| reservable as one object | **11 904 MiB** |
| **mappable** in one call | **11 857 MiB** |
| delta | **47 MiB (0.4 %)** |

⇒ `gpga_is_one_reserved_object.md` says *"the guest's advertised framebuffer size is **derived**
from the reservation that succeeded, never asserted ahead of it."* ★ **Extend it by one word:
derived from what can be *mapped*.** The guest is told 11 857 MiB; nothing else changes.

### §15.4 — What is settled, and the one thing left

| | |
|---|---|
| GPGA as ONE object | ✔ 11 904 MiB, contiguous, **1 GiB-aligned** |
| whole-object map in **one** call | ✔ at the mappable size |
| `p → base + p` | ✔ **measured** |
| base is known | ✔ `0x120000000`, returned by RM |
| §9 step 1's gate | ✔ **PASSES** |

⚠ **One caveat, recorded rather than glossed:** §12 puts the window in **every** host VAS we
create. If RM picks a **different base per space**, the base is **per-VAS** and must be tracked
per space — the arithmetic is unaffected, the bookkeeping is not. **Unmeasured.** The next run
should create two spaces and print both bases; it is one loop.

⊘ And the tiling machinery (`prove_tiled_window`) is **kept unused**: it is the fallback if a
future chip's ceiling falls well below its reservation, and §14.1's argument — that N contiguous
tiles satisfy §2 exactly as one map does — stands whether or not it is ever needed.


---

## §16 — ✔ `[MEASURED w825]` STEP 2 PASSES, AND IT EXPOSES A COLLISION THE DESIGN MUST SETTLE

```
IDENTITY_WINDOW_SECOND_VAS base=0x120000000 SAME ⇒ GPGA_VA_BASE is ONE CONSTANT for the VMM
IDENTITY_WINDOW_ESTABLISHED base=0x120000000 mib=11857
GUEST_RAM_WINDOW          mib=8192  base=0x120000000 map_ms=299 OK
```

### §16.1 — ✔ `GPGA_VA_BASE` is a constant, not a field

Two VA spaces, the same object, **the same base**. ⇒ `GPGA_VA_BASE` is **one value for the whole
VMM**, not a per-space lookup on every operand translation.

> ⊘ **CORRECTED `[fable w825]` — "one value", NOT "a `#define`".** The first wording said
> `#define`. The base is **returned by RM** (§50 level 1); a compile-time constant would demote it
> to level 6 and make it silently wrong on the next driver, chip, or host state. ⇒ **Read it back
> once at start, hold it in one place, use it everywhere.** Constant *for the life of the VM*;
> never constant *in the source*. §15.4's caveat is closed, the good way.

### §16.2 — ✔ The guest-RAM window works

**8 GiB** of guest RAM, described as **one** `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` and mapped
**whole**, in **299 ms**. ⇒ §6 is measured: a guest physical address `g` is the GPU VA
`RAM_VA_BASE + g`, and UVM's sysmem-side operands are arithmetic too.

⊘ **299 ms against the framebuffer's 1 ms**, and the asymmetry is expected rather than alarming:
pinning 8 GiB of system memory is real work where mapping already-reserved vidmem is not. It is
paid **once at VM start**. ⚠ It is also ~0.3 s of a guest's boot budget, so it belongs on the
start-up path and **never** on anything a guest can trigger repeatedly.

### §16.3 — ⚠⚠⚠ THE COLLISION, AND THE PROBE'S OWN LIMIT

**Both windows were handed `0x120000000`.** They were measured in **separate VA spaces**, so this
is RM deterministically choosing the first free address — not a conflict *yet*.

★ **But §12 puts the window in EVERY host VAS we create, and the two windows must coexist in the
SAME space.** So exactly one of these is true and **the probe did not distinguish them**:

1. RM allocates the second window **elsewhere** in the same space, both bases are known, and the
   design is unchanged — ⇒ two constants, not one.
2. RM refuses, or the caller must place one of them — ⇒ back to a **FIXED** map, which §15 spent
   eight runs discovering this VA space will not honour at an address of our choosing.

⊘ **This is the next measurement, and it is one function:** allocate **one** VAS, map **both**
the framebuffer object and the guest-RAM descriptor into it, print both bases. Until it runs,
§4's and §6's arithmetic are each proven **alone** and **not together**.

⚠ Recorded as a gap rather than an assumption, because the attractive reading — *"both work, so
both work together"* — is exactly the composition error this tree keeps paying for: every
individual mechanism in the old `gpu.rs` worked in isolation too.


---

## §17 — ✔✔✔ `[MEASURED w825]` **BOTH WINDOWS COEXIST.** Steps 1 and 2 are done, and composed.

```
BOTH_WINDOWS fb_base=0x120000000 ram_base=0x405200000 ms=306 distinct=true overlap=false
```

★★★ **One VA space. Both windows. Distinct, non-overlapping, 306 ms.** §16.3's option **1** — the
design is unchanged and there are **two constants**, not one and a problem.

### §17.1 — RM packs them by first fit, and the arithmetic confirms it

`0x120000000` + **11 857 MiB** = `0x405200000`, **exactly**. ⇒ The guest-RAM window begins where
the framebuffer window ends; RM is allocating first-fit and deterministically.

⚠ **Predictable is not guaranteed.** Read both bases back; never compute the second from the
first. A future driver that reserves something between them, or orders them differently, changes
the number without changing anything we would notice — and the whole design rests on `base` being
**right**, not on it being where we expected.

### §17.2 — The translation plane, as measured

| | |
|---|---|
| GPGA as **one** RM object | ✔ 11 904 MiB reservable, contiguous, 1 GiB-aligned |
| framebuffer window | ✔ **11 857 MiB mapped whole**, base `0x120000000`, ~1 ms |
| `GPGA_VA_BASE` | ✔ **one constant** across VA spaces |
| guest-RAM window | ✔ **8 GiB as one `OS_DESCRIPTOR`**, base `0x405200000`, ~300 ms |
| **both in one space** | ✔ **distinct, no overlap** |
| §4 `p → GPGA_VA_BASE + p` | ✔ measured |
| §6 `g → RAM_VA_BASE + g` | ✔ measured |

⇒ **Every physical-aperture operand in the design is now arithmetic on a measured base.** §9's
steps **1 and 2** are complete, on hardware, with no KVM and no guest.

### §17.3 — What remains, unchanged by this

⊘ The residual is exactly where §5 said it was and nowhere else: **virtual-aperture operands**,
which need the mirrored host VAS built by walking the guest's tables at its own invalidate. The
windows do not address it and were never meant to — they make the *physical* half free so that
the *virtual* half is the only thing left to build.

⇒ Next on §9: **step 3** — Translated for the CeUtils scrub and kernel CE, which are phys-only
and therefore pure §3 rewrite. Its gate is already printed by the existing ledger:
`forwarded=N` on tokens `0x00010001` / `0x00010004`, where w801 measured **0**, and
`execute_ours_spans` calls **= 0**.

---

## §18 — TWO GROUND TRUTHS, AND EVERYTHING ELSE IS A MAP. `[owner, w825]`

> *"Shadowing is dead right? Join as well? Only gpa (vidmem from hypervisor) and gpga (one large
> our rm allocated object) ground truths, the rest is maps for data right."*

★★★ **That is the architecture in one sentence, and it is the clearest statement of it anywhere in
this tree.** Recorded here as the invariant the rest of the design must not break.

| | ground truth | it is |
|---|---|---|
| guest **RAM** | the hypervisor's **memfd** | one `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, mapped whole at `RAM_VA_BASE` |
| guest **VRAM** | **GPGA** | **one** RM-allocated object, mapped whole at `GPGA_VA_BASE` |

⊘ **There is no third memory.** Every other structure in the system — the VA spaces, host RM's
page tables, the windows themselves, BAR1/BAR2 views — is a **map onto one of those two**. Bytes
live in exactly one place and are *reached* from several.

### §18.1 — Why this is the whole point, and what it deletes

★ A **shadow** is two memories for one address across time. It forces three things that then
cannot be got right: something must decide **when** to copy, a guest write that lands mid-copy is
**lost**, and a correct guest — not a hostile one — **races us**, because the guest does not
participate in our locking.

⇒ With one memory per address, all three questions **stop existing**:

| dead | why it cannot come back |
|---|---|
| **shadowing** | there is no second copy to be stale |
| **the join** | nothing to join — a guest FB address is an **offset into the one object** |
| **stored VA tables** | we keep no mirror; we **read the guest's own tables in place**, in GPGA |
| **auto-mapping from MMIO** | the trigger is the guest's **own** map/invalidate, never our inference |
| **per-leaf backings** | backing exists because **the object** exists |
| **promote/demote** | there is nowhere to promote *to* |

⊘ And this is why `gpu.rs`'s **seven coexisting translation designs** are not seven options. Each
was an attempt to keep a shadow correct. **The shadow is gone, so all seven answer a question that
is no longer asked.**

### §18.2 — The test, stated so a future change can be checked against it

> ★ **If a proposed mechanism requires bytes to exist anywhere that is neither guest RAM nor GPGA,
> and something must copy, sync or invalidate between them — it is a shadow, whatever it is
> called, and it is forbidden.**

⚠ This is a *structural* test, not a naming one. `install_join`, `ReachShadow`, `promote`,
`refresh`, `witness_writes` and the address table were all different names for the same thing.


---

## §19 — ⊘⊘⊘ `[fable w825]` STEP 3 IS NOT PHYS-ONLY, AND THE BUILD ORDER IS WRONG

§9 ordered step **3** (Translated for the CeUtils scrub and kernel CE) before step **4** (the
walk), on the grounds that the scrub is *"phys-only and therefore pure §3 rewrite"*. **That is
false, from ogkm's own source, and I wrote it without checking.**

`ogkm channel_utils.c` — the CeUtils channel's own plumbing is **virtual**:

```c
:489  pbPutOffset = (pChannel->pbGpuVA + (putIndex * pChannel->methodSizePerBlock));
:671  NVC8B5_SET_SEMAPHORE_A, NvU64_HI32(pChannel->pbGpuVA + pChannel->finishPayloadOffset),
:702  pSemaAddr = (pChannel->pbGpuVA + pChannel->semaOffset);
```

⇒ The scrub's **data operands** are physical on GA106 — that part was right — but its
**pushbuffer lives at a GPU VA** and its **completion semaphore is `pbGpuVA + offset`**, both in
the guest's RM-internal VA space. ★ So to **read** the guest's scrub pushbuffer at all, and to let
the GPU write its completion **natively** (§7), step 3 needs **VA→phys for that VAS** — which *is*
step 4's walk.

⇒ **`3 before 6` stands. `3 before 4` does not.** The order is **4, then 3**.

⚠ **And one more family caveat on the same step:** the scrub being physical is **not
structural**. `mem_scrub.c:152-154` flips to `VIRTUAL_MODE` when `bUseVasForCeMemoryOps` is set —
SR-IOV heavy, APM, 1:1 comptag (`mem_mgr_gm107.c:1490-1494`, `mem_mgr_ga100.c:139-145`) — and
self-hosted Hopper forces virtual (`ce_utils.c:258-262`). ⇒ On GA106/GSP it is phys; **on Hopper
it may be virtual, and then step 3 collapses into step 4 entirely.**

⊘ **A second hardware gap on the same step, worth stating before it is designed around:**
`rm.rs:66` records that `ce_copy`'s `Constant` (fill) arm was **never proven on hardware**. The
scrub is a fill. ⇒ Step 3's gate must check **the bytes actually went to zero**, not only
`forwarded>0` — a forwarded fill that writes nothing is exactly the shape §46 was written about.

---

## §20 — `[MEASURED w825]` THE BASELINE AT HEAD: 17/30, and TWO OF THEM ARE NOISE

**Box 52236011, GA106, 580.159.04, rev `95f9205f`, 45 s budget** — the same budget as w823's
15/30, so the numbers are comparable.

```
FAST_SUITE_PASS=17 FAST_SUITE_FAIL=7 FAST_SUITE_CRASH=6 NOTRUN=0
FAST_SUITE_RC=1
```

### §20.1 — ⊘ 15 → 17 IS NOT AN IMPROVEMENT. Both movers passed AT THE BUDGET.

`--uvm-mean` **45 s of 45 s**. `--map-stress` **45 s of 45 s**. Both were TIMEOUT at w823 and both
now pass **at the edge**, on a box with **15 cores at 2.4 GHz** against the previous **25**. ⇒ They
are **budget noise**, not work.

⚠ The tree already recorded this exact shape: *"`--doorbell-census` PASSED at **43 s of 45 s**. At
a 40 s budget it reads as a crash. **Record an edge as an edge.**"* ⇒ **The honest baseline is
still 15/30**, and a number that moves without a mechanism is the w797 lie in a new costume —
there `18` was our CPU doing the GPU's work.

### §20.2 — ✔ The structure is intact, and that is the RIGHT result

| | |
|---|---|
| the `forwarded=0` cluster | ✔ **still there** — `blockage-coverage`, `late-map-race`, `executor-vas`, `dictated-ring`, `ce-client` |
| `--alias-two-vas` | ✔ **PASS** — the known-positive; the gate still discriminates on real data |
| `raw client rc=1` | 2 arms, unchanged |
| group D (`concurrency`, `concurrent-fuzz`) | ✔ still hung — **as §10.1 predicts**, `kf-qemu` does not exist |

★ **Nothing built this session touches `route_of_engine`**, so an *unchanged* cluster is the
expected result and a *changed* one would have been the alarming one. The windows make the
physical half free; they do not route a single doorbell.

### §20.3 — ✔ And the harness fix earned itself

`NOTRUN=0` and **`FAST_SUITE_RC=1`**. The suite that hardcoded `FAST_SUITE_RC=0` all session now
**fails when the run fails**, and would have said `NOTRUN` + `rc=2` had a precondition been
missing instead of printing thirty phantom `TIMEOUT`s.

---

## §21 — `[MEASURED w825]` THE WALKER CAN READ THE GUEST'S TABLES THROUGH THE WINDOW — today, by a margin nothing enforces

Step 4 reads the guest's page tables **in place, through the identity window**. That only works if
the tables sit **below the window's top**. A w731 finding (`the_kernel_cannot_be_pointed_at_the_tables_where_they_are`)
said they do not — and its own EXPIRY section predicted that the one-object window would dissolve
the problem. Checked against the **live guest**, not against reasoning:

```
live guest advertised        11760 MiB      (fast_w825base_*_qemu.log: advertised=11760)
live guest VAS roots         pdb=0x2cea9c000, 0x2cea7e000  ≈ 11498 MiB
identity window top          11857 MiB      (§15)
                             ⇒ INSIDE, 358 MiB of headroom
```

### §21.1 — ⊘ My first comparison was against the wrong reference

I first placed the window against `cuda/walk/corpus/real_leaves.txt`, whose VAS roots sit at
**12026–12060 MiB** — *above* the window by 169–203 MiB. ⊘ That capture is the **host's own RM on a
full 12 GiB board**, not our guest. The guest's RM allocates tables from the top of **the
framebuffer it is told**, so the guest's roots follow the advertised size, not the card's.
⇒ The live boot answered in one grep what the corpus answered wrongly. **Measure the thing, not a
neighbour of it.**

### §21.2 — ★★★ THE INVARIANT, because today's margin is an accident

> **advertised guest framebuffer ≤ mappable identity window**

The guest's tables sit near the top of what it is told. If the advertised size ever exceeds the
window, the tables land in the **uncovered stripe at the top** — precisely the pages the walker
must read — and step 4 walks into nothing.

⊘ Today it holds because the single-store derivation yields **11760**, 97 MiB under the mappable
11857. But the obvious "fix" of deriving the size from the **reservation** (11904) would put the
top 47 MiB — the tables — outside the window. ⇒ When the fb-size derivation is ported, it must be
`min(requested, mappable_window)`, asserted at start, refused by name if violated. ⚠ This is also
the reconciling rule the audit found missing between `THE_ARCHITECTURE_v3.md:936` (*"operator asks,
or gets a refusal"*) and §15.3 (*"derived from what can be mapped"*): **the operator asks; the
answer is refused if it exceeds the mappable window.**

---

## §22 — ✔✔✔ `[MEASURED w825]` THE WALKER READS THE GUEST'S TABLES IN PLACE, IN THE ONE OBJECT

**Box 52236011, GA106, 580.159.04, bare metal, rev `4323895f`, `--cuda-window`, `ARM_RC=0`.**

```
CUDA_WINDOW_SIZED reservable_after_walker_mib=11760 (start 12288)
STORE-RESERVE ★ CONTIGUOUS and 1 GiB-ALIGNED, 11760 MiB
CUDA_WINDOW dptr=0x302000000 root=0x2cea00000 found=true runs=1 walk_us=1610
            control_found_nothing=true control_runs=0
RUNGCTL_cuda_window=PASS
```

★★★ The walk kernel, in **libcuda's** VA space (the owner's VA #1), was handed the one GPGA object
(RM-reserved, exported to a control fd, imported into the walker's own context) and walked tables
written at **11498 MiB** — the live guest's own neighbourhood (§21) — **with no relocation**, in
**1.6 ms**. `[w758]`'s *"the pointer has zero readers"* is closed.

⊘ **The control is what makes it evidence:** after the walk the root was **zeroed in the object** and
the walk repeated — **0 runs**. A walker reading a staged copy would still have found the mapping.

### §22.1 — ⊘ ORDER: the walker's context first, then GPGA

The first run reserved GPGA and then brought the walker up: `cuCtxCreate_v2 refused:
CUDA_ERROR_OUT_OF_MEMORY`. A CUDA context needs its own vidmem. ⇒ **Production order: walker
context, then reserve GPGA from what remains.**

### §22.2 — ★ That also explains the live guest's `advertised=11760`

| | reservable |
|---|---|
| no CUDA context | 11 904 MiB |
| **after the walker's context** | **11 760 MiB** |
| live guest advertised (§21) | **11 760 MiB** |

⇒ The walker's context costs **~144 MiB**, and it comes out of the guest's framebuffer. The live
single-store path already brings CUDA up first (increment 4), which is **very likely** why §21 found
the guest told 11 760. ⚠ Consistent, not proven — the two numbers were produced by different code
paths on different boxes.

⊘ **And §21's invariant simplifies.** The window no longer needs a separately-measured mappable
ceiling if the object is sized *after* the walker: the object **is** what is left, the window maps
**all of it**, and the guest is told **that**. Advertised = object = window. One number, derived once.

### §22.3 — What step 4 now has, and what it still lacks

| | |
|---|---|
| identity window in our RM VAS (§15, §17) | ✔ |
| the same object in the walker's VAS | ✔ **this section** |
| walk in place at the guest's table offsets | ✔ 1.6 ms |
| **the trigger** — guest `MMU_INVALIDATE` → enqueue → wake worker | ✘ exists only in the old tree |
| **the diff** — live tables vs our handle ledger | ✘ ledger shape exists in `storemap.rs`; not ported |
| **the maps** — FIXED slices at the guest's VAs, TLB-defer + one refresh | ✘ `raw_map_dma_slice` exists; the placement question (§19 audit: FIXED refusals *unexplained*) is **open** |
| **the `TRIGGER` clear** as the observability edge | ✘ |

⚠ The placement question is now the critical one. Step 4's output is a FIXED map **at the guest's
own VA** in a mirrored host VAS — and every FIXED attempt in §13–§16 refused, with §15.2's
explanation contradicted by §17.1. That is the next measurement.

---

## §23 — ✔ `[MEASURED w825]` FIXED PLACEMENT AT GUEST VAs WORKS. Every earlier refusal was the probe.

```
FIXED_PLACE va=0x120000000    off=0x0          EXACT
FIXED_PLACE va=0x204400000    off=0x100000     EXACT   (native oracle's semaphore region)
FIXED_PLACE va=0x2000000000   off=0x2000000    EXACT   (128 GiB, CUDA-heap shaped)
FIXED_PLACE va=0x7f0000000000 off=0x40000000   EXACT   (near a 47-bit top)
FIXED_PLACE va=0x120010000    off=0x2cea00000  EXACT   (slice at the guest's table offset)
FIXED_PLACEMENT exact=5/5 obj_mib=11760    RUNGCTL_fixed_placement=PASS
```

★ In a **default, RM-managed** VA space, 64 KiB slices of the one object land **exactly** at every
guest-shaped VA asked. ⇒ **Step 4's output shape works on hardware.** The §13–§16 refusals all ran
in a broken shape — a shared-managed space (unusable even for a plain CE copy) or a default space at
11 904 MiB, over the ceiling. §15.2's explanation *and* the audit's "unexplained" are both resolved:
**it was the probe.** ⊘ This also means the mirrored host VAS is an ordinary RM-managed space — no
`SHARED_MANAGEMENT`, no declared range.

---

## §24 — `[w826]` THE CUTOVER: what is built, what is measured, and the Translated channel's shape

**STATUS: LIVE — being built.** Owner, w826: *"not only build it, remove cpu work code so it can't
happen."* The CPU page-table walk, the CPU CE executor and the address table are ONE knot (the
executor translates through the table the walk fills; births and the old publishers read it),
so they leave in ONE cutover commit, each after its replacement is measured.

### §24.1 — Built and measured (box 52367653, GA106, 580.159.04)

| piece | commit | measured |
|---|---|---|
| walker leaf bound per aperture (sysmem leaves bounded by the host) | `08945f8a` | — |
| store imported into the walk kernel's OWN context; no staged image | `36e336c0` | selftest `CUDA_WALK=OK` |
| `plan_reconcile` + `StoreMapPort::reconcile` against the handle ledger | `36e336c0` | 5 GPU-free tests |
| `publish_walked` replaces `publish_vas_rows` (4 sites) | `0eb6997e` `40478849` | ★ **every pass `kept=N unmapped=0 refused=0`** — the GPU walk reproduces the CPU path's mappings exactly, up to 472 runs |
| births adopt the ring from the LEDGER | `beb1e1de` | — |
| PARALLEL walk from Rust (`kf_run_parallel` ported) | `067160e4` | **walk p50 424 µs** (serial 2.4 ms), max 16 ms; `uvm-mean` PASS 40 s |

### §24.2 — The Translated channel (kernel CE: RM's CeUtils scrub, UVM), and why it is NOT a decoder

⊘ The CPU executor decodes every method and resolves every operand through the table. The
Translated channel does **neither**. It copies the guest's GP entries and pushbuffer segments
into **our own** host channel and rewrites exactly two things:

1. **A CE `LAUNCH_DMA` whose SRC/DST type is PHYSICAL** — the operand becomes
   `GPGA_VA_BASE + p` (local FB) or `RAM_VA_BASE + file_offset(p)` (sysmem) and the type bit
   flips to VIRTUAL. This is RM's own `fbAliasVA` rewrite (§3).
2. **A `MEM_OP` TLB invalidate** — a split point: submit up to it, and on that submission's
   completion (an fd, not a wait on the worker's stack) walk the named root, reconcile, resume.

Everything else — semaphores, virtual operands, host methods — runs **verbatim** in a host VA
space that **mirrors the guest's kernel VA space** (the same walk + reconcile as user spaces,
now including the system proc).

★ **Reading the guest's pushbuffer is a GPU copy, never a CPU read of vidmem**: the segment's VA
is translated through our ledger (VA → store offset) and copied out with `cuMemcpyDtoH` from the
walk kernel's window pointer. A pushbuffer in guest RAM is read from the memfd directly.

⚠ **The window collision.** RM placed the identity window at `0x120000000`, where the guest's
kernel VA space also maps (its CE ring is at `0x120064000`). In Translated spaces the windows are
mapped with `DMA_OFFSET_GROWS_DOWN`, so RM places them at the top of the space, away from the
bottom-up allocations guest RM makes. A remaining collision surfaces as a named `0x51`
refusal in reconcile — never silently.

### §24.3 — Then the cutover commit deletes

`refresh_page_tables` / `sweep_cpu_pt_tables` / `PlanePtBytes` on the invalidate and sync-3
paths; the CPU CE executor's data mover and table-based operand resolution
(`WalkOperands`, `partition_ce`, `execute_ours_spans`); the address table's publish role;
`vaspace_handover`'s table cross-check; the BAR1 mirror's fill trap and the page arena.
Graded by the 30-arm suite, `cup3`/`cup8` and the LLM lane, each with `forwarded>0`.

---

## §25 — ✔✔✔ `[MEASURED w826]` **v3 GATE 3 PASSES: a guest KERNEL CE channel, Translated, on the real engine.**

**Box 52430332, GA106, 580.159.04, bare metal, no QEMU.** Rev `803da0f1`, `kf-gate3`, first run.
The harness plays RM's CeUtils channel: tables, GPFIFO, pushbuffer, semaphore and USERD in the one
store object at kernel VAs; data operands FB- and SYSMEM-PHYSICAL.

```
identity_window base=0x1fffff0000000 bytes=256MiB us=96      (GROWS_DOWN, read back)
ram_window      base=0x1ffffec000000 bytes=64MiB  us=43900   (one OS_DESCRIPTOR over the memfd)
translated fetched=3 submissions=3 splits=1 wakes=2 pumps=3 us=829
fill_zeroed · phys_copy · split_walked_the_named_root · virtual_copy_after_split ·
sysmem_to_fb · fb_to_sysmem · guest_semaphore_written_natively · gp_get_authored_on_completion=3 ·
hostile_entry_refused_by_name (Rewrite{gp:3, PeerOperand}) · hostile_entry_not_retired   ALL PASS
```

★ What is now measured rather than designed:
- **§3's `fbAliasVA` rewrite works on our own channel** — including the scrub's shape (a REMAPPED
  CONSTANT fill, `LINE_LENGTH_IN` in elements), which `rm.rs:66` had recorded as never proven.
- **§6's guest-RAM window works for CE operands in both directions.**
- **§24.2's split is correct by construction and by measurement**: the virtual copy after the
  `MEM_OP` reads through a VA the guest mapped just before it — it can only have landed if the walk
  ran after the prior work completed and before the rest was submitted.
- **§7's native completion**: the engine wrote the guest's semaphore through the mirrored kernel VAS.
- **The runner never waits**: 3 pumps (doorbell, wake, wake), each returning on its own.

⚠ **Two facts the window placement exposes, recorded before they bite:**
1. `GROWS_DOWN` put the identity window at `0x1fffff0000000` — the **top of the 49-bit space**. CE
   `OFFSET_*_UPPER` is 17 bits (`clc7b5.h:162`), so `>> 32 = 0x1ffff` is **exactly** the maximum.
   A window larger than the space's top 4 GiB-aligned slack still fits; one placed higher cannot
   exist. Our host RING must stay **below 2^40** (GP entries carry 40 bits), so it is placed
   grows-UP — where a guest kernel's own allocations also live. ⇒ **the collision is between OUR ring
   and the guest's low VAs, not the windows.** A reconcile that meets it is refused `0x51` by name.
2. The RAM window cost **44 ms for 64 MiB** (pinning); §16.2 measured 299 ms for 8 GiB. VM-start only.

⊘ Not yet covered: a guest pushbuffer IN guest RAM (read path `ram=true`), segments crossing ledger
rows, and more than one Translated channel on one worker. Next: the doorbell → worker wiring.

## §26 — ✔ `[MEASURED w826]` **v3 GATE 4 PASSES: doorbells drive workers across three Translated channels.**

**Box 52430332, GA106, 580.159.04.** Rev `65d9c452`, `kf-gate4`.

```
plane us=57100 served=227 parks=164 host_rings=180 timeslices=0 vcpu_wakes=61
per_channel (fetched, submissions, splits) = ch0:(24,24,1) ch1:(24,24,0) ch2:(24,24,0)
no_channel_died · pump_never_contended (0) · hostile_rings_produce_no_action (0 of 200000) ·
every_copy_landed (72/72) · channel_retired_and_released ×3 (GP_GET=24, sem=24 in guest RAM) ·
split_walked_once_on_a_worker                                                   ALL PASS
```

★ Measured: vCPU threads ring through `kf_trap::TrapPath` (the real trap body); two workers scan,
claim, pump and park via `try_park(seen)`; a host completion is an **internal ring** of the channel's
own token, so the pump is serialized by `BUSY` alone — **zero contended serves** proves it. Channels
live in **guest RAM** behind SYSMEM PTEs: the aperture classifier (`kf_mem::desired_from_leaves`)
maps them onto the guest-RAM object; gates 2 and 3 had hard-coded `ram:false`.

⚠ **Found on the way, and fixed:** arming a subdevice notifier is per SUBDEVICE and legal only from
`DISABLE` (`subdevice_ctrl_event_kernel.c:123-130`); the second channel's arm returned `0x40`.
⇒ `HostRm::arm_repeat`, once per session.

⚠ **The fan-out, visible in the counters:** `host_rings=180` for 72 entries. The NSI notifier is
GPU-wide and carries no identity, so every completion flags EVERY host ring's event fd and rings
every channel — including idle ones. ⇒ Next: **one session event fd**, and on its readiness ring
only the tokens with a fence **in flight** (a set the rings maintain). The wake cannot be narrower
than "someone finished"; the ring can be narrower than "everyone".

# The single-store branch — plan of record

**STATUS: LIVE (2026-09-14, w721).** Branch `single-store`. Owner's goal, verbatim:

> *"get in that branch the raw client in its full test suite passing under kayfabe guest, with the
> PTX parser and scratchpad update. One GPGA store, one RM object, no more fake fb, no more
> bar1/bar2 traps, no more populate on fault (if thats possible). the rest of untouched constraints
> remain."*

Governing constraints: **20** (the GPU walker), **21** (one CUDA program, Turing→Blackwell, format
as setup data), **22** (we do not lie about the aperture). §15 and §19 are **superseded**; §18 is
**satisfied by construction**. Everything else in `THE_CONSTRAINTS.md` stands untouched.

## ⊘⊘⊘ THE SEQUENCING RULE — deletions come LAST

Build the new path, prove it, **then** delete the old. ⚠ If deletion rides along, the tree is
broken across a long stretch with **no working intermediate and no way to bisect** which half broke
the raw client. This tree's whole method is measured increments; a big-bang rewrite abandons it
exactly where it is most needed.

## ⊘⊘⊘ READ THIS FIRST — the live status board, 2026-09-15

⚠ **The sections below have accumulated corrections as SIBLINGS rather than folded above what they
correct**: there are **three** `### 3.` sections and **two** `### 6.` sections, and file order no
longer matches execution order. This block is authoritative; a section below that disagrees with it
is stale.

| # | increment | status |
|---|---|---|
| 1 | VM-lifetime scratchpad isolate | ✔ **BUILT** — `KAYFABE_SCRATCHPAD`, booted, 11904 MiB reserved |
| 2 | the reserved object | ✔ **BUILT** (came with 1) |
| 4 | CUDA in the scratchpad isolate | ✔ **BUILT** — `KAYFABE_SCRATCHPAD_CUDA`, `CUDA_WALK=OK` |
| 5 | the format seam | ✔ **BUILT** — no bit position left in the kernel |
| — | the **crossing** (§3's prerequisite) | ✔ **BUILT & PROVEN** — `DEVICE_VIEW=OK`, ruling w727b |
| **6** | **walker → publish path** | ◐ **STEP 1 ✔ / STEP 2 ✔ BUILT, ONE MEASUREMENT OUTSTANDING** — shadow clean (`compared=65 disagreements=0`); the **swap** is `KAYFABE_WALK_SHADOW=swap`, w732. ⊘ See the correction under §6: it does **not** retire the host walk |
| **3** | BAR1/BAR2 as device views, the switch | ○ **UNBLOCKED, NOT STARTED — and it comes AFTER 6** |
| 7 | the deletions | ○ not started; licence is the **guest suite**, not one workload |
| 8 | the raw client's full suite, in the guest | ○ not started |

### ★★★ THE ORDER IS 1,2,4,5 → **6** → **3** → 7 → 8 — and 6-before-3 is FORCED

⊘ `RegPlane::window_leaves` builds its reader over `PlaneMem::fb` — **the BAR1/BAR2 walk reads page
tables out of the very `FbStore` the switch replaces.** Move the store first and every page-table
read becomes a **48 MiB/s CPU read through a scarce aperture**, which is the ~10-minutes-a-boot cost
that was the whole reason the two-world split existed. ⇒ **The switch would re-create the problem it
exists to remove.**

⊘ And the intuitive reason for the coupling is **wrong**, which is why it was got backwards: *"the
kernel needs the tables in GPU memory"* — they are in a host memfd today, reading at ~3.7 GB/s.
**Residence is not the constraint; the READER is.**

### ⊘ The falsifier list below is STALE in one entry

`two_worlds_split::a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` is
**unbuildable**, not pending: it asserts BAR1 and BAR2 are *two memories*, and §15 is superseded —
there is **one world**. ⇒ The gate is its **inverse**, `..._is_the_page_bar2_reads`, which passes
today and **fails the moment anyone gives BAR1 its own store** — exactly the accident a one-path
`page_backing` switch produces.

## The increments, in order

### 1. A VM-LIFETIME SCRATCHPAD ISOLATE  ⟵ **BUILT 2026-09-14, behind `KAYFABE_SCRATCHPAD`**

> ★ `crates/kayfabe-qemu-raw/src/scratchpad.rs`, spawned in `Regs::create_probed` — the
> composition root, once per device, at PCI realize. **Owned by the shell, not by a `Proc`**:
> `Spine::install_isolate` is keyed by `(ProcId, GpuId)` and there is no `ProcId` that means
> *"the VM"*; its `IsolateId` proc field is `u32::MAX` so it can never alias a live proc's.
> ⊘ It spawns **beside** the device, never through it, so the three tests that keep isolate
> spawns guest-caused (`tests/tests/isolate_spawn_is_guest_caused.rs` and its two neighbours)
> stay true rather than being edited to accommodate this.


⊘ `[surveyed w721]` The device model is **entirely per-proc** — `procs: BTreeMap<ProcId,
RankedMutex<Proc>>` (`kayfabe-rt/src/device.rs:15`) — and isolates are spawned **lazily, on first
guest touch**. **Nothing today lives for the VM.**

But the reserved object, the CUDA context and the walker must all exist **before the guest's first
instruction**. ⇒ This is the foundation everything else hangs off, and it is new plumbing in the
device spine.

⚠ Spine work means **lock ranks** (R1/R3) and the no-blocking-under-locks invariants. The isolate
spawn path is already a recorded slow-trap site (`the_vcpu_spawns_an_isolate_and_blocks_on_its_socket`),
so spawning at VM start is *also* a latency win — it moves a known 1.6 s vCPU stall off the guest's
path entirely.

### 2. THE RESERVED OBJECT

> #### ⊘⊘⊘ CORRECTED 2026-09-14 (increment 1, built) — **THE VERB BELOW IS THE WRONG VERB, AND
> #### IT WOULD HAVE REFUSED THE BOOT FOR A REASON THAT IS NOT CAPACITY.**
> The paragraph below says *"the verb already exists (`Request::AllocVidmem`, wire tag 19)"*.
> **It does not.** Read from the two bodies:
> - `RmBackend::alloc_vidmem` → `RmConnection::alloc_device_local` (`rm.rs:2690`):
>   **`ATTR_CONTIGUOUS_VIDMEM`, `alignment: len`**.
> - `RmConnection::reserve_gpga` (`rm.rs:2673`) — written *for* `gpga_is_one_reserved_object.md`
>   and, until increment 1, reachable only from the ladder binary **inside the child**:
>   **`ATTR_NONCONTIGUOUS_VIDMEM`, `alignment: 4096`**.
>
> ⇒ A contiguous, **11.8 GiB-aligned** 11.8 GiB request fails on merely *fragmented* free
> memory. ⚠ The failure is the dangerous kind: the boot is refused, and the refusal **means
> something other than what it says** — the one call whose refusal is supposed to read as
> *"there is not enough video memory"* would instead be reporting fragmentation and alignment.
> `reserve_gpga`'s own doc comment already said both differences "were wrong for it"; nobody
> had joined that to this plan.
>
> ★ And note what made it survivable: `largest_reservable_mb` probes with `reserve_gpga`, so a
> probe-then-`alloc_vidmem` implementation would have **measured 11 808 MiB as available and
> then failed to allocate it** — a mismatch between the size advertised and the size held, which
> is exactly the shape this design exists to remove.
>
> ⊘ **Neither verb was on the wire.** `largest_reservable_mb` is an inherent method on
> `HostRmBackend`, not on the `RmBackend` trait, so the parent could not call it either. ⇒
> increment 1 adds two trait methods with named-refusal defaults, wire tags **27
> `ReserveGpga`** and **28 `LargestReservableMb`**, and reply tag **14 `Megabytes`**.
> ★ The probe runs in the **child**, one round trip: it is ~8-13 real multi-gigabyte RM
> alloc/free pairs, and driving the bisection from the parent would put a dozen IPC brackets
> where one belongs.

One reservation of the derived size, owned by the scratchpad isolate. `largest_reservable_mb`
already derives the size (11808 MiB measured on a 12 GiB GA106). ⇒ Advertise **what was
reserved**, never assert ahead of it (`gpga_is_one_reserved_object.md`).

**⊘ The rule is enforced by an ARM, not by default.** `gpga_is_one_reserved_object.md` says
*"If that fails, the VM does not start."* That is right for the product and wrong for the
measurement: a device that refuses to realize produces **no teardown census, no guest, and no
answer to what the host would have given us** — it produces a QEMU that exits with a status.
⇒ `KAYFABE_SCRATCHPAD=on` asks the question and `=require` enforces the answer, and the census
line is printed **before** the refusal is consulted so a refusing boot still names which step
refused.

> ## ⊘⊘⊘ THE ORDER IS WRONG — **6 MUST PRECEDE 3** (found 2026-09-14, building the switch)
>
> `RegPlane::window_leaves` — the BAR1/BAR2 walk both the premap and the mirror depend on —
> builds an `FbStoreReader { fb }` over `PlaneMem::fb` and hands it to
> `kayfabe_mmu::walker::decode_subtree`. ⇒ **the page tables are read out of the same
> `FbStore` the switch replaces.**
>
> So the moment that store becomes the reserved object, **every page-table read is a CPU read
> of video memory** — `[measured]` 48 MiB/s, through a BAR1 aperture §22 item 3 showed is
> scarce. That is the *"~10 minutes per boot"* cost w721 cites as the whole reason the
> two-world split existed. **Flipping the store before the walk moves GPU-side re-creates the
> problem the single store exists to remove.** §7 (w723b) already says it from the other side:
> the BAR1 capacity worry *"dissolves once our own PT reads move GPU-side"*.
>
> ⊘ **And the intuitive reason for the dependency is wrong**, which is worth naming because it
> points the arrow the other way: *"the kernel needs the tables in GPU memory"* — no. They are
> in a host **memfd** today, which reads at ~3.7 GB/s, so the kernel can be fed by uploading
> them. **Residence is not the constraint.** The constraints are the walk's byte source *after*
> the flip, and §6's own shape mismatch.
>
> ⇒ **§6 (route (a): the report also carries the visited page list) first, then §3.**

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ **UNBLOCKED 2026-09-14; the CROSSING is built and proven**

> ✔ **The ruling landed and the crossing is working code.** `Request::ExportDeviceView` is back
> on the wire (request tag **30**, reply tag **15**; 25/12 stay retired) with
> `ReleaseDeviceView` (31), and `[measured, rev 814c02c1]` a real boot reports
> **`DEVICE_VIEW=OK mmap_len=0x1000 sentinel_roundtrip=true released=true`** — the scratchpad
> isolate armed a view of the **reserved object**, the node crossed by `SCM_RIGHTS`, the VMM
> mapped it, **closed the descriptor**, and the mapping survived. Evidence:
> `traces/single_store_crossing/`.
>
> ★ Condition 2 of the ruling is **measured** rather than asserted: the sentinel round-trip runs
> through a mapping whose descriptor is already gone.
>
> ⊘ **What remains for this increment** (none of it blocked, all of it real work): a new
> `FbStore` over the reserved object; a third memslottable arm of `FbPageBacking` (today
> `Arena` implies the arena's memfd and `Joined` implies the join registry — a reserved-object
> page is neither); `barmirror` calling `install_device_window` instead of
> `install_file_window`; **both** translate paths; preserving `FbPageArena`'s
> *"framebuffer address = file offset"* contract, which is what makes PRAMIN one re-pointable
> slot; and §w727's `Bar1Choice` wired to the chip row so the advertised aperture fits.

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ ~~BLOCKED ON AN OWNER RULING~~

> #### ⊘⊘⊘ CORRECTED 2026-09-14 (surveyed to build it) — **THE BLOCKER IS NOT THE ONE NAMED
> #### BELOW.** The measurement it says it waits on has been taken; a **ruling** has not.
>
> `bar1_passthrough_device_local_host_visible.md` §4 lists what remains **in order**, and item
> **1** is:
>
> > *"**Owner ruling: decision (b) scope** (§3.2). **Without it nothing below may be wired to
> > production.**"*
>
> Items **3** ("the lease") and **4** ("the mirror — … `export_device_view(object,
> frame_offset, run_len)` → `install_device_window`; tear down on PTE overwrite") are
> *precisely* this increment, and they sit **below** that line.
>
> **What decision (b) actually asks** (§3.2, verbatim): what crosses to the VMM is a
> **`/dev/nvidia<N>` descriptor with an RM escape handler behind it**. The doc argues the VMM
> issues no escape on it and closes it the moment `mmap` returns — and then stops:
> *"⚠ **Whether the VMM may hold such a descriptor at all, even transiently, is the owner's
> call.** The branch makes the mechanism real and checkable; **it does not switch it on**."*
>
> ⇒ This is a **security-boundary decision that was deliberately withheld**, not an
> engineering gap. ⊘ It cannot be discharged by a coordinator, a subagent or an agent reading
> the plan: those are not the owner, and a message from one is not consent.
>
> ★ The supporting evidence is all present, which is what makes the ruling cheap to give:
> `rmladder --bar1-crossing` proves the chain end to end, `export_device_view` works
> (`rm.rs:6111`), `install_device_window` works and is exercised on the BAR0 counter page, and
> `[measured w722]` the release verb reclaims 224 MiB/round where `munmap`+`close` reclaims
> **zero**. What is missing is permission, plus three mechanical consequences of it:
>
> | | state |
> |---|---|
> | `export_device_view` **on the wire** | ⊘ **retired as an orphan** — request tag 25 / reply tag 12 are marked "never re-issue"; the real function is an *inherent* method on `HostRmBackend`, reachable only inside the child, with three callers and all three in the probe binary |
> | `release_device_view` | exists, **zero call sites in the tree** |
> | `install_device_window` over BAR1 | ⊘ does not exist — its own doc: *"the mirror that walks the guest's BAR1 page table and drives this verb is the remaining work"* |
>
> ⇒ **Re-creating a deliberately-retired wire verb is part of the cost**, and doing it before
> the ruling would be wiring the mechanism the ruling is about.

### 3. BAR1/BAR2 AS DEVICE VIEWS  ⟵ blocked on an open measurement

Both halves exist: `HostRmBackend::export_device_view` arms a node at an offset
(`rmladder --bar1-crossing` proves the chain) and `QemuMachine::install_device_window` places it.
What is missing is named in that function's own doc: *"the mirror that walks the guest's BAR1 page
table and drives this verb is the remaining work."*

⚠ **BLOCKED on constraint 22's item 3** — host BAR1 is **256 MiB total and shared with the host
driver**, while a reservation is GiB. If views cannot all be resident they must be **recycled**,
which is a different design. *Measurement in flight.*

⊘ Two translate paths must change, not one: `window_page_backing` (premap) **and** `window_phys`
(`fb_read`/`fb_write`, the trap path). Fixing only the first leaves trapped accesses reading the
fake fb and **looks like a partial success**.

### 4. CUDA IN THE SCRATCHPAD ISOLATE  ⟵ **BUILT 2026-09-14, behind `KAYFABE_SCRATCHPAD_CUDA`**

> ★★★ `crates/kayfabe-cuda` (the committed PTX, the launch ABI mirrored, a `dlopen`ed DRIVER
> API) + a **second, glibc-linked isolate image** chosen by `IsolateId.proc == u32::MAX`, with
> CUDA brought all the way up in `build_backends` **before** `sandbox::enter` and the two
> §w724d probes run **after** it. Gate: `KAYFABE_SCRATCHPAD_CUDA=on`, a **peer** of
> `KAYFABE_SCRATCHPAD` rather than a third arm of it — the two are orthogonal and a boot must
> be able to arm either alone.
>
> ⊘⊘⊘ **THE BLOCKER IS MEASURED AND IT IS MORE GENERAL THAN §w724d STATES.**
> `[measured 2026-09-14, locally, no GPU]` a **musl static-pie** binary's `dlopen` returns
> `NULL` with `dlerror()` = *"Dynamic loading not supported"* — for `libcuda.so.1`,
> `libc.so.6` and `libm.so.6` **alike**. It is not *"libcuda is the wrong kind of shared
> object"*; it is *"there is no dynamic linker in that process"*, and the refusal arrives
> before any question about CUDA is asked. ⇒ a different **build** is the only fix, exactly
> as §w724d prescribes — and no GPU was needed to establish it.
>
> ★★★★★ **AND THE SETUP-DATA TRANSCRIPTION WAS WRONG FOUR TIMES.** The descriptor the host
> hands the kernel (§21's *"the format is setup data"*) is ~120 numbers, and a differential
> against the `.cu`'s own builder caught four errors on its first run — two of which
> (`dir[3].leaf_ps`, `dir[4].leaf_ps`) would have made the host and the kernel disagree about
> whether 512 MiB and 2 MiB pages exist at all, and one of which (`pde_ap_map[0]`) would have
> behaved identically and differed **silently, forever**. ⇒ **§21's "derive the descriptor
> from `GmmuFmt`" is not a tidiness item.** Until increment 6 does that, the differential is
> what makes the transcription safe to rely on.


`libcuda` + `cuModuleLoadData` (where the PTX JIT runs) **before** the isolate drops privilege —
CUDA is lazy, and every lazy path is one that fails after the drop. No other isolate loads CUDA
(~135 MB of mappings and hundreds of ms of context creation). **One walk isolate per VM.**

### 5. THE FORMAT SEAM  ⟵ REQUIRED BEFORE ANY DELETION (constraint 21)

The host walker is format-polymorphic (`fmt: &dyn GmmuFmt`); `cuda/walk` hardcodes VER2. Deleting
the host parsing before the kernel has the seam **silently caps the product at Ada**, and Blackwell
is goals 1 and 10.

> ## ⊘⊘⊘ CORRECTED 2026-09-15 (w731, building the live half) — **THE NUMBER BELOW IS NOT
> ## THE ONE IT TURNS ON, AND THE MEMFD ROUTE IS NOT THE ROUTE.**
>
> The block below says the live half *"turns on one number"* — `store_refused`, whose zero
> would make it safe to hand the isolate the arena's memfd. `[measured w730]` it is zero, and
> that is a true measurement of a real hazard. ⊘ **But it was never the binding one.**
>
> **The kernel addresses page-table pages as offsets into ONE FLAT WINDOW** — `KfArgs::win =
> {base, len}`, and every dereference is bounds-checked against `win.len`
> (`cuda/walk/kf_walk.cu:346,357`). So a window that answers for the guest's tables **at their
> own GPGA must be as long as the highest table page's address**.
>
> - `[measured w730]` the page arena's high-water on a full raw-client boot is
>   `span_pages=3087533` — **~11.78 GiB up a 12 GiB framebuffer**. The guest's own RM puts its
>   tables at the **top**.
> - `[corroborated]` `cuda/walk/corpus/real_leaves.txt`, w725's capture of a **real** driver's
>   tables, has its five address-space roots at `0x2efa4c000 .. 0x2f1cac000` — the same place.
>
> ⇒ An identity window is an **11.8 GiB device allocation on a 12 GiB board**, competing with
> the guest's own forwarded video memory. ⚠ It is not affordable, and **no assertion about
> `store_refused` changes that** — the blocker is the WINDOW, not the arena.
>
> ★ **w725 hit the same wall from the other side and had already answered it**
> (`cuda/walk/kf_real_tables.py`): *"the tables live across ~12 GiB of framebuffer, which no
> test buffer can hold … table **pages** are relocated into a compact arena and the **address
> field of directory entries** is rewritten to point at the new home."* w731 lands that live —
> `GmmuFmt::relocate_entry` + `kayfabe_mmu::walkshadow::build_image`.
>
> ### ⇒ AND THE HAZARD THIS BLOCK EXISTS FOR IS DISSOLVED, NOT ASSERTED AWAY
>
> Because the image is **built by the parent**, its bytes are read through `FbRead` — the same
> authoritative byte source the host walk itself reads, which answers correctly whether a page
> is in the arena or on the heap. ⇒ **there is no second image that could be partial, and no
> counter to assert at every use.** The memfd grant is not built; it would have saved wire
> bytes and nothing else. The arena line is still printed beside the census for continuity.
>
> ### ⚠ AND THE CONSTRAINT THE PLAN CHECKED IS NOT THE ONE THAT BINDS EITHER
>
> The ✔ below records a feared blocker checked and false: the sweep's EXECUTE phase holds no
> lock, so a synchronous isolate round trip there is not an R1 violation. **True, and it is a
> different question from which THREAD it runs on.** `sweep_cpu_pt_tables` is called from
> `ring_inline`'s settlement and from the register-write settle-before-birth path, and
> `[goal 4, w656-w660]` a doorbell can be rung **by one dword store on the vCPU** — where a
> round trip blocks a vCPU. ⇒ w731's observer **declines by name** on a vCPU thread and the
> census counts it (`skipped[on_vcpu=N]`); the off-vCPU `refresh_page_tables` path is where the
> shadow actually lives.
>
> ### ★ EXPIRY — relocation is TRANSITIONAL (§w724g)
>
> The sparsity is a property of **today's `SparseFb`**. §3 makes GPGA one reserved
> device-local object which `gpga_is_one_reserved_object.md` has the scratchpad map **whole, at
> a fixed base** — *"an address is `X + gpga_offset`"*. ⇒ **after §3 the window is identity and
> `build_image` is retired.** ⚠ Expected end state, not a promise: nothing has built that
> mapping yet.
>
> ## ⊘⊘ §6 STEP 1 — the COMPARISON is built; the LIVE half turns on one measurement
>
> `[2026-09-15]` `kayfabe_mmu::walkshadow` lands the half that makes the swap safe: the host
> walk's leaves as `walkdiff::Run`s, the comparison against the kernel's report, a census by
> kind, and the known-positives its zero depends on — every kind made to fire by name, both
> **vacuity** arms pinned (a shadow that never ran and two empty sets render as VACUOUS, never
> as agreement), and the three canonicalisation properties checked over `[w725]`'s **real
> GA106 capture** rather than over a fixture: idempotence, self-agreement, and
> **order-independence** (the host emits depth-first, the kernel per-thread).
>
> ⊘ **What is compared is narrower than "everything", and the census line says so.** The host
> walker does not decode volatile, privilege, atomic-disable or `KIND`; the kernel does.
> `COMPARED_FLAGS` is the intersection, and the clean verdict names what it did not cover.
>
> ### ✔ One feared blocker checked and FALSE
>
> I expected the sweep to run under the device lock, making a synchronous isolate round trip an
> R1 violation. It does not: `SharedDevice`'s sweep is plan/execute/commit and the site is
> marked **"EXECUTE — no lock"** (`device.rs:4449`). ⇒ an isolate call there is legal, and no
> deferred queue is needed.
>
> ### ⊘⊘⊘ The real blocker: THE FAKE FB IS PARTLY A MEMFD AND PARTLY THE HEAP
>
> To run the kernel at refresh time the guest's page tables must be reachable by the **GPU**.
> They live in `SparseFb`, whose `pages: HashMap<u64, FbPage>` holds each page *"on the heap
> **or** in the arena"* — and `arena_refusals` counts every time the arena refused and a heap
> page was made instead.
>
> ⇒ Granting the isolate the arena's memfd (the `with_guest_ram` shape, and within its
> precedent) would hand it **some** of the framebuffer and **silently miss the rest**. A walk
> over that follows a zero PDE and reports *nothing* — so the shadow would report
> `missing_in_kernel` for mappings that are not missing, and the census's whole value is that
> it can be believed.
>
> ⚠ This is the same class `arena_read_refusals` was added for in w585: *"the store and the
> file disagree about a frame"*.
>
> ★ **It turns on one number, and the boot already prints it**: `fb_arena_census`'s
> `arena_refusals`, reported at teardown by `barmirror`. **Zero on a real boot** ⇒ the memfd
> route is viable with a checked assertion beside it. **Non-zero** ⇒ the route is dead and the
> tables must move to the reserved object first, which is step 2 — i.e. the ordering correction
> one level further down.

> ## ⊘⊘⊘ CORRECTED 2026-09-15 (w732, BUILDING the swap) — **THE CONSUMER NAMED BELOW IS NOT
> ## THE CONSUMER, AND THE SWAP DOES NOT RETIRE THE HOST WALK.**
>
> Both §6 blocks below say the leaf path is *"`leaves` → `Settlement` → `AddressTable::bind`"*.
> `[read from the bodies, w732]` **`kayfabe_fwd::commit_pt_decode_with` — the only thing the
> sweep commits through — touches `SubtreeDecode::leaves` NOWHERE.** What reaches the address
> table is `SubtreeDecode::decodes[*].1.leaves`, the **per-page** leaves, by way of
> `ReachShadow::observe` → `settle` → `apply_settlement_as`. The flattened `leaves` field has
> exactly **three** readers in the tree and all three are elsewhere: `ceresolve`, the BAR
> mirror's `window_leaves`, and the shadow's own comparison.
> ⇒ ⚠ **A swap that replaced only `leaves` would have changed NOTHING and would still have
> read as done** — a green boot, a clean census, and the host walk still deciding every bind.
> The substitution therefore rewrites the leaves **inside each page's decode**.
>
> ### ★★★★★ AND THE HOST WALK CANNOT LEAVE THE PATH — THE DEPENDENCY RUNS THE OTHER WAY
>
> `walkshadow::build_image` takes `(pdb, &[PtPage])` — **the host walk's own `visited` set** —
> and needs each page's **level** to know its entry size and geometry. ⇒ **the host walk is a
> PREREQUISITE of the kernel, not an alternative to it.** There is no boot, today, in which
> the kernel runs and the host walk does not.
> ⊘ This inverts how the swap reads: `on` → `swap` moves *where a published leaf's target,
> aperture and writability come from*. It does **not** produce one walker, and it does **not**
> license §7's deletion. ★ It expires with **§3**: an identity window needs no page list.
>
> ### ⊘⊘ AND WHAT REMAINS FOR THE KERNEL TO OWN IS NOW EXACTLY NAMEABLE
>
> `visited`, `children`, `sparse`, `invalid` — the **reachability vocabulary**
> (`Admit::{Witnessed, Swept}`, `PublishedUnbind`). The kernel's report is `MapRun`s, *with no
> pages in them at all*. That is the whole of §6's residual shape mismatch, and the deltas-vs-
> state question below is **still unpicked** — the swap did not need to pick it, because it
> substitutes into a structure the host walk already produced.
>
> ### ⚠ AND THE SWAP IS OBSERVATIONALLY NEUTRAL BY CONSTRUCTION — SO GRADE IT ACCORDINGLY
>
> It substitutes **only where the two walkers agree**, and under agreement the two leaf sets
> are the same set. ⇒ **no boot can distinguish a live swap from a decider nobody consulted**,
> and a parity number or a green client says nothing about it. The known-positive is
> deliberately offline: `tests/tests/walk_swap_decides.rs` drives a decider that disagrees on
> purpose and requires the changed binding to land in `AddressTable`. A boot's job is the
> other two facts — `decided>0`, and the client unharmed.

### 6. WIRE THE WALKER INTO REFRESH  ⟵ **BLOCKED ON A SHAPE MISMATCH, NOT ON WIRING**

> #### ⊘⊘⊘ SURVEYED 2026-09-14 — **"kernel launches replace the host walk" is not a wiring
> #### job**, and the reason is a type, not an integration.
>
> **What publishes today** (`kayfabe-fwd/src/ptdecode.rs`, `commit_pt_decode_with`):
> `decode_subtree` returns `SubtreeDecode { leaves, visited: Vec<PtPage>, decodes: Vec<(PtPage,
> PageDecode)> }`, and **all three halves are consumed**:
> `visited` → `vas.pt_meta` + `learned_pages` → `publish_pt_pages`; `decodes` →
> `ReachShadow::observe(PtPage, &PageDecode)`, whose witness/swept gating is keyed on **pages**;
> `leaves` → `Settlement` → `AddressTable::bind`.
>
> **What the kernel returns**: `Vec<MapRun>` → `walkdiff::Run { va, gpga, len, flags, class }` —
> **coalesced runs, with no pages in them at all**.
>
> ⇒ Bridging them means deciding what happens to the **reachability shadow**, `pt_meta`,
> `learned_pages` and `publish_pt_pages` — i.e. to the admission rule (`Admit::{Witnessed,
> Swept}`) and the unbind policy (`PublishedUnbind`). That is a **design change to the
> publication contract**, and it is the same question §6 already flags as unsettled
> (deltas vs current state) arriving from the other side.
>
> ⊘ **And the two halves are half-present in code already**, which is worse than either:
> `walkdiff`'s module doc states the kernel reports *current state* and the host diffs, while
> `kf_diff_kernel` is still launched on every `refresh`, and `HF_RESYNC` / `generation` /
> `acked_generation` / `RunOp::{Map,Unmap,Remap}` are all a **delta** vocabulary. **Pick
> deliberately** is still the instruction, and nothing has picked.
>
> ⊘ **Scope hints do not exist on either side.** `KfScope` is defined and never constructed;
> `nscope` is hardcoded `0`. Of the three invalidate sources, two carry a `Pdb` and **none
> carries a VA range**; source 3 (the UVM kernel channel) carries no `Pdb` either, and
> `shim.rs` records that by construction it never can.
>
> ⚠ The dependency on §3 is **not** residence — the guest's tables are in a host memfd today
> and a memfd reads at ~3.7 GB/s, so the kernel could be fed by uploading them. It is the
> shape above.

### 6. WIRE THE WALKER INTO REFRESH

Kernel launches replace the host walk. Scope hints from the three invalidate sources; degrade to a
full walk. ⚠ **Open design choice, unsettled:** does the kernel return **deltas** (needs a vidmem
shadow) or **current state** (host diffs against its Rust model, no shadow)? The format doc says
deltas; the design conversation landed on state. **Pick deliberately** — it decides whether the
shadow, the generation handshake and the forge-unchanged attack surface exist at all.

### 7. THE DELETIONS — only now

fake fb (`SparseFb`), the per-leaf join, the page arena, the demand-fill `BarMirror`, the CE
table-copy, the host-side PT parsing. ⊘ **Keep the host walker as a TEST-ONLY oracle** — it is what
made the 1212-leaf differential meaningful. Two implementations in *production* is a bug factory;
two where one is the *oracle* is how you know the other is right.

⇒ ~**4 500 LOC** deleted outright, ~**2 700** more reduced, against ~1 000 added. Concentrated in
the four most defect-dense crates.

### 8. THE GATE — the raw client's FULL suite, in the guest

`scripts/bench/rmladder_suite.sh`, all 30 arms. ⊘ *"A test that exists but nobody runs is not a
useful test."* Host first, then guest; a bare-metal pass with a guest fail indicts kayfabe
(`bare_metal_pass_guest_fail_indicts_kayfabe`).

## The falsifiers that must flip

| | today | after |
|---|---|---|
| ⊘⊘⊘ `two_worlds_split::a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` | `#[ignore]`d, **RED** | ~~GREEN~~ ⇒ **THIS ROW IS WRONG AND PREDATES w721.** That test asserts BAR1 and BAR2 are two memories; the single store makes them one. It cannot go green without re-introducing the second memory the reserved object deletes. **Deleted in increment 7.** The single store's own falsifier is the inverse and is PASSING today: `a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads`. |
| `IGNORED_ALLOWANCE` in `run_full_suite.sh` | **2** | back to **1** |
| LLM parity | **0.20x** | the number this is all for |

## ⚠ What is NOT in scope

Anything outside the constraints above. In particular the **50x bulk-placement defect**
(`to_device`, 0.8 host cores for 28 s, GPU 1.8% busy) is a **separate** bug in the H2D
copy-forwarding path that residence work will **not** fix — do not let it be absorbed into this
branch's story either as a cause or as a success.

## ★★★★★ w723b — BAR1/BAR2 BECOME PURELY GUEST-FACING

**Owner, 2026-09-14:** *"so bar1 becomes unused for ourself, if the tables is in ptx cuda? So
bar1/bar2 is then only for mmio cpu mappings for the guest right."* ★ Correct, and it retires a
whole category of pressure.

**Every CPU view of video memory we hold exists to READ THE GUEST'S PAGE TABLES.** That is what
PRAMIN and the BAR2 window do on our behalf. ⇒ With the kernel reading them **GPU-side** at
~360 GB/s, **we never need a CPU window onto video memory again.**

⇒ BAR1, BAR2 and PRAMIN become **apertures the guest's CPU uses**, which we serve with device
views. The budget stops being contended between us and the guest.

| consumer of host BAR1 | measured |
|---|---|
| the guest's own BAR1 mappings | **3.6 MiB** (912 pages, LLM workload) |
| our CUDA context | **~3 MiB** |
| **total** | **~7 MiB of ~254** |

★ This also **dissolves the self-starvation hazard** recorded in constraint 22: the guest's views
and our CUDA context were only in competition because **both** were CPU views of video memory. Now
only one of them is.

### ⇒ Open choice: put the REPORT BUFFER in host memory

The kernel could write the report straight into **host** memory over PCIe — 32 KB is nothing — so
reading it costs **zero BAR1** and **no CE readback path at all**. That takes our own aperture
consumption down to just the CUDA context.

⚠ The trade: it would be the **only host memory mapped in the CUDA context**, so a wild kernel
write could reach it. ⊘ Bounded, though — it is a buffer the kernel writes **by design**, the host
**validates it regardless** (format doc §3), and the exposure is one mapping rather than an address
space. ⇒ **Recommended**: strictly less machinery than a vidmem report plus a readback.

## ★★★★★ w724g — GATES EXPIRE. Don't carry the system you pivoted from

> **Owner, 2026-09-14:** *"you should not build cruft of older systems we have pivoted from like
> trapped bar1/bar2 if its already untrapped for a while. So during a pivot from A to B you can add
> a gate to ensure both keep working, and if it then boots and raw client works with the full suite
> then the older one can be unwired."*

★ Accepted. The sequencing rule (*deletions last*) guards against deleting **before** proving; this
guards against **never deleting after**. They are not in tension — together they say *prove, then
delete promptly*.

### ⊘ The cruft trap is not the gate — it is UNWIRED-BUT-STILL-COMPILING

Dead code that builds **looks maintained**. Someone will later "fix" it, or a reviewer will assume
it is load-bearing and design around it. ⇒ **Unwire and delete in the same change**, never as two.

⊘ And gates cost *during* the pivot: each doubles the state space under test. Two live gates is a
four-arm matrix, and this tree already grades arms by hand. **Keep the count small.**

### ★★★ THE MECHANISM: a gate is created WITH ITS EXPIRY CONDITION

Every gate names, in its own doc comment, the condition under which it is **deleted** — e.g.
*"deleted when the guest suite is green with the arm on"*. ⇒ It cannot quietly become permanent,
and whoever finds it later does not have to guess whether it is still needed. Same discipline this
tree already applies to rulings, where **a ruling's date and expiry are part of the citation**.

### ⚠ PUSHBACK — trapped BAR1/BAR2 is not yet cruft, and the distinction matters

`TRAP_FILLS=0` says traps **do not fire**. It does not say the trap path is **unreachable**, and
those are different claims (§w721b's *"zero in practice vs impossible by construction"*).

★ Today the trap path is the **backstop that makes premap-completeness a SOFT property**. Delete it
and completeness becomes a **hard correctness requirement** — which is exactly the *"no populate on
fault"* question whose **mechanism** is settled (§w721b) and whose **coverage** is not.

⇒ Delete it, but **deliberately**: make the trap path **refuse by name**, boot, confirm the refusal
never fires. One boot, and it converts *"zero in practice"* into *"proven unreachable"* — which is
what licenses the deletion. ⊘ Housekeeping-on-the-assumption-it-is-dead is how a soft property
becomes a hard one without anyone deciding to make it so.

### ⚠ And "the full suite" means the GUEST suite

`[measured]` host **30/30**; the guest had genuine failures (`--concurrency`, `--engines`, and a
`--gpu-info-sweep` timeout that wedged the device and cascaded 25 arms). ⇒ A host-green suite is
the easy way to declare victory early. **The gate for unwiring is the guest suite.**

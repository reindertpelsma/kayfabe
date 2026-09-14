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

## The increments, in order

### 1. A VM-LIFETIME SCRATCHPAD ISOLATE  ⟵ the foundation, and it does not exist today

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

One `alloc_vidmem` of the derived size, owned by the scratchpad isolate. The verb already exists
(`Request::AllocVidmem`, wire tag 19); `largest_reservable_mb` already derives the size
(11808 MiB measured on a 12 GiB GA106). ⇒ Advertise **what was reserved**, never assert ahead of
it (`gpga_is_one_reserved_object.md`).

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

### 4. CUDA IN THE SCRATCHPAD ISOLATE

`libcuda` + `cuModuleLoadData` (where the PTX JIT runs) **before** the isolate drops privilege —
CUDA is lazy, and every lazy path is one that fails after the drop. No other isolate loads CUDA
(~135 MB of mappings and hundreds of ms of context creation). **One walk isolate per VM.**

### 5. THE FORMAT SEAM  ⟵ REQUIRED BEFORE ANY DELETION (constraint 21)

The host walker is format-polymorphic (`fmt: &dyn GmmuFmt`); `cuda/walk` hardcodes VER2. Deleting
the host parsing before the kernel has the seam **silently caps the product at Ada**, and Blackwell
is goals 1 and 10.

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
| `two_worlds_split::a_framebuffer_page_written_through_bar1_is_not_the_page_bar2_reads` | `#[ignore]`d, **RED** | **GREEN**, attribute deleted |
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

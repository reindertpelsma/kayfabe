# R30 — THE CPU VIEW: which object can carry it, and why the named mechanism cannot

**STATUS: LIVE BUT RESCOPED, 2026-08-27.** Forward-ported from branch `fb-join` (`2fe5f39`)
unchanged; the measurements below stand. ⊘⊘⊘ **What changed is the SCOPE OF THE TITLE.**

> ### ⊘⊘⊘ RESCOPED 2026-08-27 — THIS DOC ANSWERS *"can `ExportBacking` carry it"*, NOT
> ### *"does vidmem have a CPU view"*. The second reading is FALSE and it cost the placement fix.
> The title asks *"which object can carry the CPU view"* and §0.1 answers for **one crossing
> mechanism**: `ExportBacking`'s `HostDeviceMemory` source, which always refuses. That result
> is real and re-verified. ⊘ But downstream it was compressed into
> *"vidmem — with **no CPU view**"* on `FbLeafBacking::Vidmem`, and **that sentence is false**.
> ★ Vidmem has a CPU view by a **different mmap route**: `NV_ESC_RM_MAP_MEMORY` (`0x4E`)
> registers an mmap *context* against a caller-supplied descriptor and the caller `mmap`s
> **that descriptor** — RM selecting **the device node for a BAR address** and the control
> node for sysmem, and refusing the wrong kind outright. Cited, with `ogkm-580` line numbers,
> in our own `kayfabe-abi/src/submit.rs:46-66` — we specified the route and never issued it.
> ★ **`nvkvm-pv` ships it**, isolate included (`src/qemu/nvkvm_isolate_handlers.c:3618`,
> `src/guest/nvkvm_uvm_ext.c:585`). §0.1's own **reason 1 names this route** and objects only
> to handing the fd across the isolate/VMM seam — a statement about *crossing*, not existence.
> ⚠ Load-bearing, not cosmetic: sysmem operand placement is the **14.80×** factor in the
> large-kernel perf gap, so the compressed reading has been pricing the data plane.
> ⊘ **Still open, and not licensed by this rescope:** which process holds the BAR mapping, and
> whether a VMM holding one keeps *"only the unprivileged isolate touches the GPU"* true.
> Reason 2 (dma-buf gated on `PDB_PROP_GPU_ZERO_FB`, an integrated-part property) still kills
> **dma-buf** as the crossing descriptor on every discrete card we target; reason 3
> (`DeviceBackingNotPlaceable`) is **our own** refusal, not a hardware fact.

This is a **probe report**: what it measured on a real GA106 is not affected by the port, and
its conclusion (which object can carry a guest-reachable CPU view) still stands.
⚠ Its successor `fb_join.md` §5.12 **has** changed — the bind moved after the install and now
declares `kayfabe_mmu::BackingBytes::JoinsGuestWindow`. Read that doc's own STATUS block
before building on this one. ⊘ Nothing here was re-run.

**Rung:** `cpu-view` / `3437903`, based on `ae73f6b` (`w228`, the second crossing).
**Question, from the brief:** close *"two memories"* before anything executes — route
`Request::ExportBacking` to the framebuffer path and join the host object's CPU view to the
shell's framebuffer storage, so one object has two views.

**Answer: the named mechanism cannot exist, and the reason is measured rather than argued.**
What was built is the instrument that settles it: a host-side ladder rung (`R30`) that, on a
real GA106, establishes the premise, watches the crossing refuse **by name**, and proves the
**corrected** join in both directions.

⊘ **This rung does NOT close the two-memories gap.** It establishes which chain closes it.
Read §0.5 before reading any green line.

Companions: `fb_leaf_crossing.md` §5.9 (the rung this answers), `guest_ram_crossing.md` §5.8
(the first crossing, whose shape turns out to be the right one here too),
`isolate_vmm_fd_crossing.md` §12 (the owner's decision (b), which is what refuses).

---

## 0. ★★★★★ WHAT I REFUTED FIRST — INCLUDING THE BRIEF THAT ORDERED THIS RUNG

### 0.1 ⊘⊘ REFUTED — the brief's named mechanism. `ExportBacking` can never carry that object's view

The brief says: *"`Request::ExportBacking` already exists on the wire and is not routed to
this path. Route it, and join the host object's CPU view to the shell's framebuffer
storage."*

Routing it is a **deterministic refusal**. `ExportBacking` carries an `ExportSource`, and the
only source that could name `w228`'s vidmem object is `HostDeviceMemory`, which is
**always** `RmError::NotExportableAsMemory`:

```
crates/kayfabe-isolate-host/src/rm.rs:2807-2814
crates/kayfabe-isolate/src/lib.rs:498-520      (the type: "Two variants, and the second one ALWAYS refuses")
```

`[SOURCED]` It is a **decision with three independent cited reasons**, each sufficient
(`rm.rs:2780-2806`, `kayfabe-isolate/src/lib.rs:324-355`):

1. the only object whose `mmap` yields a host GPU page is `/dev/nvidia<N>` with a registered
   mapping context — a **character device**, and RM recomputes `secInfo.privLevel` from the
   **caller** on every escape (`ogkm-580: arch/nvalloc/unix/src/escape.c:304`), so the same
   descriptor is unprivileged in the isolate and privileged in a root VMM;
2. NVIDIA's own dma-buf — the one non-RM descriptor that could cross — gates CPU mapping on
   `PDB_PROP_GPU_ZERO_FB` (`ogkm-580: arch/nvalloc/unix/src/osapi.c:5609`,
   `kernel-open/nvidia/nv-dmabuf.c:1246-1250`), an **integrated**-part property. On every
   discrete card this project targets a dma-buf of device memory cannot be `mmap`ped at all;
3. our own memory plane refuses the result independently —
   `kayfabe_linux_raw::GuestWindow::place` rejects `Backing::DeviceFile` with
   `RawError::DeviceBackingNotPlaceable` (`crates/kayfabe-linux-raw/src/window_unsafe.rs:209-213`).

`[MEASURED]` and this rung did not take that on trust: `R30` hands `export_backing` the
**live** vidmem object and watches it refuse. It did, by name, on both runs (§3).

★ And the docs pre-empt the obvious workaround in as many words (`rm.rs:2804-2806`):
*"⊘ Do not 'fix' this by copying the device pages into a `memfd`. A copy is not a mapping."*

### 0.2 ✔ CONFIRMED — the brief's premise, and it was worth measuring

The brief asked me to confirm that `alloc_vidmem`'s objects are CPU-mappable as created
(`rm.rs:2421` → `alloc_device_local`, no `NVOS02_FLAGS_MAPPING_NO_MAP`, unlike the
guest-RAM descriptor path at `rm.rs:1549`).

`[MEASURED]` **True.** `NV_ESC_RM_MAP_MEMORY` succeeded and 16 384 words round-tripped
through the mapping (§3). ⊘ Absence of a refusing flag is not the presence of a mapping, so
this needed the host to say it, and now it has.

### 0.3 ⊘ REFUTED — "the fix is wire the view up, not recreate the objects"

Both halves of that sentence are wrong in the same direction. The view **exists** (§0.2) and
it is a mapping of `Backing::DeviceFile` (`rm.rs:1455-1461`), which is precisely the thing
that cannot cross (§0.1) and that `GuestWindow::place` refuses. There is no wiring that
reaches from it to the shell.

⇒ **The fix IS to recreate the objects**, on the `Fabricated` + `OS_DESCRIPTOR` chain — the
arm of `export_backing` that is *designed* to succeed (`rm.rs:2769-2779`). §4.

### 0.4 ⊘⊘ REFUTED — and here the tree contradicts itself, in two documents that are both right about their own half

`FbStore`'s own docs nominate the convergence (`crates/kayfabe-device/src/fbwin.rs:220-232`):

> *"the convergence is an implementation of **this** trait that delegates to the isolate, and
> `RegPlane::set_fb` is the seam it is installed through … `&mut self` … the eventual
> production implementation is a **connection** to an isolate and every access is a round
> trip"*

`[SOURCED]` **That implementation cannot be installed.** Every `FbStore` call site holds the
register plane's FSM mutex — `plane.rs:2881` takes `self.state.lock()` and `:2891` calls
`s.fb.read(...)`; the write path is the same at `:2916`/`:2982` — and the executing gate
`tests/tests/unranked_locks.rs:56-59` classifies that lock as:

> *"★★★ THE HAZARD. The register-plane FSM mutex, taken on the vCPU MMIO trap and held
> across the entire policy chain. ⊘ **NOTHING may block beneath it**: a wait here stalls
> every vCPU's register access, **and the R1 witness will not say so**."*

The doorbell port is outside `state` for exactly this reason and says so
(`plane.rs:1073-1079`, `:3109-3115`), and `Worker::fb_read` **panics** if a ranked lock is
held (`crates/kayfabe-isolate/src/lib.rs:2028-2031`).

⇒ ★★ **A per-access delegating store is barred**, and the bar is invisible to the R1 witness
— the one instrument that would normally catch it. The join must be a **mapping**, not a
connection: a memcpy into an `mmap` blocks on nothing. This is the single fact that decides
the successor's shape, and it is why §4 is not the shape `fbwin.rs` nominates.

### 0.5 ⊘⊘ REFUTED, MINE — what this rung does NOT do, stated before any green line

- **The two-memories gap is OPEN.** No leaf is joined; `SparseFb` is untouched; nothing is
  installed at any framebuffer physical address. `R30` allocates its own objects, its own
  memfd and its own VAS, and frees them.
- **`cup2` is not expected to pass and was not run against a join.** This rung routes no
  doorbell and points no engine anywhere; there is no mechanism by which `cuCtxCreate` could
  progress. `Route::NotACopyEngineChannel` stands exactly as at `w228`.
- ⚠ **The brief's bar point 3 — "a control boot with the join unarmed" — is NOT met in that
  form, and I am not going to call the substitute equivalent.** There is no join to unarm.
  What exists instead is (a) the `--fb-view-negative` arm, which is a *stronger* control over
  the property under test because it isolates sharing itself (§0.6), and (b) a stamped
  baseline boot at `ae73f6b` on this bench (§3.3). Neither is a guest-visible disagreement
  measurement, because nothing guest-visible changed.

### 0.6 ★★ The negative control, and the question `w228` §0.1 taught us to ask first

*Which line do I expect this control to execute?*

- **The crossing control** → the `let ... else` arm of `RmBackend::export_backing`
  (`rm.rs:2808-2813`), which destructures the source and returns `NotExportableAsMemory`
  before any host call. It is handed a **live** object it can evaluate, so it is a
  proposition its target can refuse — unlike `w228`'s first control, which asked
  `back_fb_leaf` about an address it never walks.
- **The join control** → `Backing::PrivateAnonymous`'s arm of the mapping's `mmap` argument
  computation (`crates/kayfabe-linux-raw/src/mapping_unsafe.rs:344-347`), yielding
  `MAP_PRIVATE|MAP_ANONYMOUS` instead of `MAP_SHARED`. ⊘ **Not "a second memfd"**, which
  would be a tautology — two different files obviously hold different bytes. The control
  changes **only** the one property that makes two mappings one memory, and everything
  either side of it is the same code.

---

## 1. ★★★ THE STRUCTURAL FACT THAT DECIDES EVERYTHING

Three things, none of which is negotiable at this layer:

| | fact | citation |
|---|---|---|
| 1 | The shell's framebuffer is a **private heap `HashMap`** — `SparseFb { pages: HashMap<u64, Box<[u8;4096]>> }`. No `mmap`, no `memfd`, **nothing shareable with any process**. | `crates/kayfabe-device/src/fbwin.rs:566-582`; installed once at `crates/kayfabe-qemu-raw/src/shim.rs:5733` |
| 2 | The isolate is a **different process** — `clone`d into six namespaces, `exec`'d from a sealed memfd, reachable only over socketpairs at fixed fd numbers. | `crates/kayfabe-isolate-host/src/lib.rs:36-61`, `src/isolate.rs:1154-1230` |
| 3 | **Every** guest framebuffer access is a VM exit. BAR1/BAR2/PRAMIN are all `memory_region_init_io`; nothing is ever mapped into the guest. | `qemu/hw/misc/nvkvm/nvkvm.c:694-715`, `:740-747`; `crates/kayfabe-device/src/plane.rs:2653`, `:2881` |

⇒ There is **no** mechanism today by which the isolate, or the host GPU, could see one byte
of the emulated framebuffer — which is what the *"Two memories"* comments record
(`crates/kayfabe-fwd/src/lib.rs:1878-1884`, `crates/kayfabe-rt/src/completion_watch.rs:450-453`,
`crates/kayfabe-qemu-raw/src/shim.rs:3837`).

★ Fact 3 is the encouraging one and is easy to misread as a cost: because the access already
traps, joining the framebuffer costs **no new exit**. What it cannot afford (§0.4) is a
*blocking* call under the plane lock — so the join must replace `SparseFb`'s pages with
shared ones, not replace its *lookup* with an RPC.

---

## 2. What was built

One ladder rung, no production path touched.

- **`HostRmBackend::prove_fb_view`** (`crates/kayfabe-isolate-host/src/rm.rs`) — four facts
  in one chain, each reported separately because each has a different cause:
  1. `alloc_device_local` (byte-for-byte what `alloc_vidmem` issues) → `map_cpu` → per-word
     store and load. **The premise.**
  2. that same live object → `export_backing(HostDeviceMemory)`. **The crossing control.**
  3. `mint_fabricated` → **two** `MappedRegion`s of the one `memfd` → per-word pattern
     written through each and read through the other. **The join, both directions.**
  4. `alloc_os_descriptor` → `raw_map_dma(..., Some(at))`. **The GPU view.**
- **`FbViewJoin::{Shared, Private}`** — the arm selector, `OsDescSeed`'s shape.
- **`rmladder --fb-view-probe` / `--fb-view-negative`**
  (`crates/kayfabe-isolate-host/src/bin/rmladder.rs`).

### 2.1 ★★ Two ordering decisions that are load-bearing

- **Direction 1 runs BEFORE the descriptor exists.** That is the *establishment* direction —
  bytes the guest already wrote must be visible to what RM is about to describe — and the
  real establishment copy runs at exactly that instant.
- **Direction 2 runs AFTER `alloc_os_descriptor` and the FIXED `map_dma`.** That is the
  *engine* direction, and running it after the pages are pinned and in the GPU VAS makes the
  answer about the memory RM is now holding rather than about a mapping RM has never seen.

### 2.2 ⊘ The pattern is per-word, never a repeated constant

Word *i* is `base + i`. A read that returned a zero fill, a truncated length, or a different
buffer's bytes cannot match — whereas a whole-buffer compare against one repeated word passes
on any single correct word. The reported `words_compared` comes from the loop that counted
(`ViewCompare::agrees` refuses `words_compared == 0`), which is `OsDescEvidence`'s own
already-shipped-once defect, not re-derived.

---

## 3. MEASURED — real GA106, bench `vh2`

`[measured 2026-08-11, bench `vh2`, RTX 3060 GA106 (GPU-45cf77eb…), host driver 580.159.04,
euid 0]`
**Binary stamped `3437903bb422566f28d2228ba5c4b8e15c5746f0`** — printed by the binary itself
as `REV_UNDER_TEST`, from `KAYFABE_BUILD_REV` set to `git rev-parse HEAD` at build time, not
read off a file that claims to record it.

Evidence: `traces/real_ga106/rmladder_r30_fb_cpu_view_real_ga106.txt` and
`…_negative_real_ga106.txt`.

### 3.1 ★★★★★ The positive arm — all four facts

```
★     R30 premise        = the vidmem object `alloc_vidmem` mints IS CPU-MAPPABLE:
                           NV_ESC_RM_MAP_MEMORY succeeded and 16384 words round-tripped
ok    R30 neg control    = export_backing(HostDeviceMemory) on that SAME live object
                           → NotExportableAsMemory, BY NAME
      R30 guest→host     = all 16384 words AGREE (0xa19a5a5b → 0xa19a5a5b)
      R30 host→guest     = all 16384 words AGREE (0x043ffffe → 0x043ffffe)
★     R30 JOINED         = ONE fabricated backing, TWO independent mappings, 65536 bytes
                           agreeing in BOTH directions, described to RM as an OS_DESCRIPTOR
                           and placed at 0x00007f0000000000 AS ASKED
PROBE_RC=0
```

⇒ The premise holds; the crossing is shut **by name**; and the corrected chain carries bytes
in **both** directions and reaches the host GPU's MMU at a dictated VA.

### 3.2 ★★★★★ The control — watched to fail, and it failed at the right word in both directions

```
      R30 guest→host     = DISAGREE at word 0 (got 0x00000000, want 0xa19a5a5b) of 16384
      R30 host→guest     = DISAGREE at word 0 (got 0xa19a5a5b, want 0x043ffffe) of 16384
ok    R30 CONTROL FIRED
CONTROL_RC=0
```

★★ **Read the second line — it is the strongest single signal here.** The control's
guest-side read did **not** return zeros. It returned `0xa19a5a5b`: the *direction-1* pattern,
still sitting in the private pages this run wrote it into, because direction 2's write went
to the shared memfd and never reached them. A control that merely returned zeros both ways
would be consistent with a mapping that was never written at all; this one demonstrates both
views are live, hold different bytes, and are being read by the same loop.

⊘ The premise and crossing-control lines are **identical** across the two runs, which is
what a control that changes only the join must produce.

### 3.3 The bench baseline

`traces/guest_boots/run_base_ae73f6b_w228_{qemu,dmesg,probe}.log` — a stamped `ae73f6b` boot
on `vh2` (35 dmesg lines, 31 `NVRM`), establishing that this second bench reproduces the
`w228` tree before anything on this branch changed. ⊘ It is a device-open boot, not a `cup2`
run: it carries no GR census and is **not** a `w228` §4 reproduction.

---

## 4. ★★★★★ THE SHAPE THE SUCCESSOR MUST TAKE — and it is the FIRST crossing's, not the C's

The C double-maps a host **vidmem** object: CPU side into QEMU's own address space, GPU side
FIXED at the guest VA, *"both sides share ONE coherent host object"*
(`C: nvkvm_gpu_emul.c:8454-8459`, alloc at `:7286-7294`). That works **because the C is
monolithic** — QEMU holds `/dev/nvidia` itself. Here the same object's CPU view is a
`Backing::DeviceFile` mapping inside the isolate and cannot reach the VMM (§0.1).

The chain `R30` proved instead, with every step already in the tree:

```
export_backing(Fabricated, len)          → a sealed memfd, minted in the isolate
   the fd rides Reply::Backing            ← the ONE reply that may carry a descriptor
   the VMM adopts it into ExportRegistry  ← already built and tested
mmap it in the isolate                    → the GPU-side view
   alloc_os_descriptor(that mapping)      → NV01_MEMORY_SYSTEM_OS_DESCRIPTOR
   map_gpu_va(vas, memory, len, at=leafVA)→ DMA_OFFSET_FIXED, placement CHECKED
mmap it in the shell                      → the guest-side view: FbStore storage for
                                            [leaf.phys, leaf.phys+len)
```

★ It is `PinGuestRam`'s chain (`map → describe → map_gpu_va`,
`crates/kayfabe-isolate/src/lib.rs:2247-2311`) with the **memfd's owner inverted**: guest RAM
crosses VMM→isolate at spawn on `GUEST_RAM_FD`; this crosses isolate→VMM on the reply. No new
descriptor mechanism, and **no device fd anywhere** — decision (b) is honoured rather than
circumvented.

### 4.1 ⚠ The cost this shape pays, named rather than discovered later

**The leaf becomes host SYSMEM, not host VIDMEM.** The engine reaches it over PCIe instead
of from local framebuffer. That is a **performance** divergence from the C and from `w228`,
not a correctness one, and it is the identical trade the first crossing already makes for the
guest's ring. ⊘ It is also not optional: vidmem is exactly the memory that cannot carry a
guest-reachable CPU view, which is the whole of §0.1.

★ A second, smaller cost: `ChildExports::mint` uses `SharedRam`, whose `F_SEAL_SHRINK` is
load-bearing here for the reason its own docs give — a shortened file under a live mapping is
`SIGBUS` in **both** processes (`crates/kayfabe-isolate-host/src/export.rs:63-68`).

### 4.2 ★★ The establishment bridge is REQUIRED — the brief asked, and the answer is yes

The C copies unconditionally (`C: :8281-8290`), not behind a flag. Here it is required for a
reason the C's shape does not have: the fabricated backing is a **fresh** `memfd`, zero-filled
by `ftruncate`, while the guest's bytes for that leaf are already in `SparseFb`. Installing
the join without the copy would present the engine a blank pool for a leaf the guest has
written — the exact defect `w228` §3 records, moved one layer along.

⊘ It must run **at install, before the join is live**, and it must read `SparseFb` **directly**
rather than through the joined store, which by then answers for that range. The owner's
*"mapping after execution seems racy to me"* is honoured structurally: after the copy there is
one memory, so there is never a merge to get wrong.

### 4.3 ⊘ What §4 still does not answer

- **Extent.** `R30` uses 64 KiB of its own choosing; a leaf is 2 MiB (`w228` §4.1), and one
  `SharedRam` per leaf is the safe unit, not the efficient one. The C's run-coalescing and
  2 MiB re-cut (`C: :8472-8477`) needs a leaf enumeration this port does not have.
- **`FbStore`'s shape.** A store that holds joined `mmap`s plus `SparseFb` for everything else
  must keep `resident_frames`/`page_origin`/`is_resident` honest across both halves, and
  `device_reset` must forget joined ranges too (`fbwin.rs:336-349`) — that is a cross-life
  leak if it is missed.
- **Lifetime.** The FB re-back gap `fb_leaf_crossing.md` §3.1 records is inherited unchanged.

---

## 5. What the next rung inherits

1. **The chain is proven on hardware** (§3.1) — build the plumbing, not the proof.
2. **A named, measured refusal** to build on the vidmem object's view (§0.1, §3.1).
3. **A hard constraint on where the join may live** (§0.4): a mapping, never a connection,
   because `Mutex<PlaneState>` tolerates nothing that blocks and the R1 witness will not say so.
4. ⊘ **Not the doorbell gate.** `fb_leaf_crossing.md` §3 stands unchanged.

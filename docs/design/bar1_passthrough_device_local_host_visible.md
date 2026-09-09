# ★★★★★ BAR1 passthrough — `DEVICE_LOCAL | HOST_VISIBLE`, not trapped at all

**STATUS: LIVE — SECOND INCREMENT LANDED, 2026-09-09 (branch `w393-bar-passthrough`): the
DEMAND-DRIVEN MIRROR, for BAR1 *and* BAR2, behind two QOM arms (`bar1-passthrough`,
`bar2-passthrough`). See §7. §2.7 below is SUPERSEDED by §7.1 and says so in place. The
first increment (§3, the crossing and the placement) is unchanged and still has no production
caller; the owner AUTHORISED the fd crossing in that shape on 2026-09-09 (§3.2 is answered).**
The owner's stated architecture (2026-09-09): *"bar1/2 should not have traps and just
passthrough"*, named precisely as the Vulkan memory-property pair **`DEVICE_LOCAL | HOST_VISIBLE`**.
This doc records (1) how BAR1/BAR2 are realized **today**, with file:line; (2) the design that
reaches the semantic; (3) the increment on this branch; (4) what remains, and which parts are
**owner rulings** rather than engineering. Nothing here has booted a guest. Two facts the design
rests on were **readings** when this was written and are turned into measurements by
`kayfabe-rm-ladder --bar1-crossing` (§6) — the result of that run is recorded in §6.4 when it exists.

⚠ Grade every sentence below against the owner's acceptance test, verbatim:
1. a guest CPU store to a BAR1 offset takes **no VM exit** — no `bar1_reads`/`bar1_writes` moves;
2. the host engine reads the bytes that store wrote **without any join, alias, publish or carry**;
3. there is **no `SparseFb` copy** of that range at all.

---

## 1. How BAR1 and BAR2 are realized TODAY — measured from the tree, not inferred

⊘ The prior claim *"BAR1/2 appear untrapped"* (`RESUME_HERE_w392_overnight.md:420`) was **false**
and was withdrawn in `4cf454dc`. This section is the verification.

### 1.1 The QOM glue — `qemu/hw/misc/nvkvm/nvkvm.c`

One region table, one constructor, and BAR1/BAR2 are **both `NVKVM_KIND_TRAP`**:

| row | port | PCI BAR | kind | constructor | callbacks | where |
|---|---|---|---|---|---|---|
| `nvkvm-bar0-regs` | 0 | 0 (32-bit) | TRAP | `memory_region_init_io` | `nvkvm_trap_ops` | `nvkvm.c:881` |
| `nvkvm-bar1-window` | 1 | 1+2 (64-bit) | **TRAP** | `memory_region_init_io` | `nvkvm_bar1_ops` (`:861`) | `nvkvm.c:889` |
| `nvkvm-bar2-window` | 2 | 3+4 (64-bit) | **TRAP** | `memory_region_init_io` | `nvkvm_bar2_ops` (`:702`) | `nvkvm.c:897` |
| `nvkvm-msix` | 3 | 5 | MSIX | container | — | `nvkvm.c:899` |

- Every BAR1 access enters `nvkvm_bar1_read` (`:753`) / `nvkvm_bar1_write` (`:836`), which record
  it (`nvkvm_bar1_record`, bounded 16 + uncapped `bar1_touches`), print the GP_PUT-shaped ones
  live, and hand it to the archive as `KAYFABE_BUS_BAR_FB`.
- Every BAR2 access enters `nvkvm_bar2_read/write` (`:671/:679`) → `KAYFABE_BUS_BAR_INST`.
- ★ The table's own comment says why BAR1 became a trap (§16.18, `:882-888`): it used to be a
  discarding `RESERVATION` (`(void)val;`) and the three writes that **are** the guest's whole
  submission handshake were being destroyed. The fix made it trap, and `bar_is_unbacked_reservation`
  (`:1278`) then answers **no** for BAR1, which is load-bearing: the archive refuses to install a
  memslot over a trapping row (`kayfabe-vmm-qemu/src/lib.rs:1196`, `WINDOW_IN_A_BACKED_BAR`).
- The realize-time whole-BAR shadow (`window-size` property) is explicitly **refused** for BAR1
  (`:1602-1616`) with the §16.18 reasoning: *"a slot there would answer the guest out of memory the
  framebuffer store and the page walk cannot see, and a read back would agree with it."*
- `nvkvm_report_audit` prints a trap-status table per row from `memory_region_find` (`:2050-2100`)
  — so a booted log **states** which rows are IO vs RAM; on every committed boot BAR1 reads `IO`.

### 1.2 The archive — where the trapped access goes

- `RegPlane::read/write` attribute the access by the window the address model resolved:
  `crates/kayfabe-device/src/plane.rs:2896` (`bar1_reads`), `:3296` (`bar1_writes`),
  `:2905`/`:3271` (`bar1_faults`).
- The BAR1 translation is `RegPlane::bar1_phys` (`plane.rs:3089-3131`): a GMMU page walk rooted at
  **our own** `GspStaticConfigInfo.bar1PdeBase` (`kayfabe-device/src/lib.rs:587-608`; published,
  not received — `kbusPatchBar1Pdb_GSPCLIENT` re-roots the guest's BAR1 walker onto it), over
  page-table pages read out of the emulated framebuffer store, refusing by name a foreign aperture
  (`BAR1_FOREIGN_APERTURE`) and a read-only PTE on a write.
- The bytes land in `SparseFb` (`crates/kayfabe-device/src/fbwin.rs:917`): a `HashMap<frame, Box<[u8;4096]>>`
  in the VMM's heap — unless the range is a **joined leaf**, in which case they land in a per-leaf
  `memfd` the isolate minted and also mapped (`SparseFb::joined`, `install_join` at `:1185`).
- BAR2 has its own root model (`crates/kayfabe-device/src/bar2.rs`, `UPDATE_BAR_PDE` fn 70) and the
  same store.

### 1.3 Why the whole fb-join chain exists — it is downstream of the trap

Because guest CPU writes land in **our heap**, not in memory the GPU can read, a frame the engine
touches must be *joined* (`join_fb_leaf`: mint memfd → mmap in the isolate → `OS_DESCRIPTOR` →
`map_gpu_va(FIXED)` → memfd crosses to the VMM → `SparseFb::install_join` copies the heap bytes in),
joins must be *released* on remap, releases must *carry bytes*, stale joins must be *revoked*, and
aliases/extents must agree. Every defect fixed in the last 24 h (`git log --oneline -40`: the dropped
fill head, the stale binding, the ghost peer, the extent mismatch, the grow-in-place) is one
instance of *two memories for one byte*. The join is the trap's consequence, not a feature.

### 1.4 What the C artifact did — NOT a precedent

`archive/nvkvm/src/qemu/nvkvm_gpu_emul.c:9807` and `:9818` are `memory_region_init_io` too: the C
**trapped BAR1 and BAR2 as well**. It got away with it because its green run was a control-plane
result (`m2cexec=0`: the CPU did the copies), so the GPU never needed to read bytes the guest CPU
wrote through BAR1. Kayfabe forwards real engine work, so the same trap is what costs every join.
⇒ This design is greenfield; nothing in this project passes a BAR through.

---

## 2. The design — one allocation, two viewers, zero interception

### 2.1 The invariant

> **A framebuffer frame the guest names through BAR1 is backed by ONE host vidmem allocation from
> the moment it is leased, and that allocation is mapped twice: into the guest's BAR1 guest-physical
> range (a KVM memslot, so a guest CPU store lands in it with no exit) and into the host GPU's
> address space (`map_gpu_va` FIXED at the guest's own VA, so the engine reads the same bytes).
> Nothing copies, joins, carries or revokes bytes, because there is never a second copy.**

`SparseFb` holds bytes the GPU cannot see. Under this invariant there are no such bytes for a
BAR1-named frame — criterion 3 holds by construction, not by discipline.

### 2.2 The pieces, and which already exist

| piece | what | exists? | where |
|---|---|---|---|
| **GPU view** of a vidmem object at the guest's VA | `map_gpu_va(FIXED)` over an `alloc_vidmem` object | ✔ | `kayfabe-isolate-host/src/rm.rs` (`FbLeafBacking::Vidmem` → `VerbPlan::PublishVidmem`, `kayfabe-fwd/src/lib.rs:2980`) |
| **CPU view** of that object on the host | `NV_ESC_RM_MAP_MEMORY` (0x4E) registered on a fresh `/dev/nvidia<N>` node + `mmap` of that node | ✔ ~20 sites | `RmConnection::map_cpu` / `map_cpu_windowed_on` (`rm.rs:2456`, `:2502`) |
| **The crossing**: that CPU view reaching the VMM | hand the **armed node** (not the object, not the control fd) to the VMM over `SCM_RIGHTS`; the VMM `mmap`s it once at offset 0 and closes it | ◐ child table existed with no caller (`ChildExports::mint_armed_node`, `export.rs:146`, 2026-08-27); **the verb is this branch** (§3) | `RmBackend::export_device_view` |
| **Placement** of a device mapping inside a guest window | `MAP_FIXED` of the node into the window a memslot names | ✘ refused by our own `GuestWindow::place` (`DeviceBackingNotPlaceable`, `window_unsafe.rs:213`); **a separate verb is this branch** | `GuestWindow::place_device_view` |
| **The memslot** over a sub-range of the pure-MMIO BAR1 region | the foreign-slot mechanism (`host_execution_plane.md` §1.5): the BAR stays `memory_region_init_io`, so KVM's listener never creates a slot for it and ours cannot be clobbered | ✔ mechanism; **the device-backed variant is this branch** | `QemuMachine::install_device_window` |
| **Permission** for the archive to shadow BAR1 | `bar_is_unbacked_reservation(BAR1) == yes` | **this branch**, behind a QOM property | `nvkvm.c` `bar1-passthrough` |
| **The mirror**: guest BAR1 page table → memslots | walk `bar1PdeBase`, one slot per contiguous run of PTEs onto one leased object; install on PTE publication or on first-touch; tear down on PTE overwrite (observed through the BAR2/BAR0-window traps, which stay) | ✘ **remaining** (§4) | shell |
| **The lease**: GPGA frame → host vidmem object | "GPGA owns memory, procs lease it" (owner ruling); lease at first naming by *any* path; 2 MiB granule | ✘ **remaining** (§4) | core/shell |

### 2.3 Why the guest's BAR1 page directory still exists, and what it becomes

A real BAR1 is a GMMU-translated aperture: the guest's RM allocates BAR1 VAs dynamically
(`kbusMapFbAperture` → the BAR1 VAS allocator) and writes PTEs naming FB physical addresses. We
publish the root (`bar1PdeBase`) and the guest writes the tables — through BAR2 or the BAR0 window,
**both of which stay trapped**. So the BAR1 PT is still the authority on *which frame sits at which
BAR1 offset*; the passthrough does not remove it, it **mirrors** it into memslots. There is no
"identity BAR1" shortcut: RM will not map BAR1 1:1 and we control neither side of its allocator.

⇒ Each BAR1 memslot is `[bar1_base + va_run, len) → mmap(armed node for the leased object at
frame_offset)`. The trap callbacks remain as the **named** fallback: an access that reaches them
under the armed arm is a `BAR1-PASSTHROUGH MISS`, counted, printed, and served correctly (slow) —
never a silent fallback (§3.4).

### 2.4 The classes — passthrough for the common case, refused BY NAME for the rest

| BAR1 PTE names | can it be `DEVICE_LOCAL \| HOST_VISIBLE`? | disposition |
|---|---|---|
| aperture **Vidmem**, frame leased to a host vidmem object | **yes** — this design | memslot over the crossed view |
| aperture Vidmem, frame with **no lease yet** (guest wrote it only through BAR0-window/BAR2 into `SparseFb`) | yes after the lease; the lease point is *first naming by any path* | until the lease lands: named miss, served from the store; **this is the residual join-shaped step and it is bounded to page-table/instance pages the guest never BAR1-maps** |
| aperture **Sysmem** (guest RAM) | `HOST_VISIBLE` trivially; `DEVICE_LOCAL` is false by the guest's own declaration | memslot aliasing guest RAM at the BAR1 GPA (`SharedFile{guest ram fd, offset: gpa}`), engine side via the guest-RAM pin (`OS_DESCRIPTOR`). Today refused as `BAR1_FOREIGN_APERTURE`; **deferred, named** |
| **host BAR1 exhausted** (the view cannot be armed: `NV_ESC_RM_MAP_MEMORY` refuses) | no — a hardware budget (§2.5) | named miss `HOST-BAR1-EXHAUSTED`; served through the isolate's windowed CPU view (`map_cpu_windowed_on`) |
| the guest's own **page-table / instance-block** pages (written via BAR2/BAR0 window, read by our walker) | not needed — never BAR1-mapped by RM | stay in the store; under the lease they may also live in vidmem, read through a windowed CPU view |

### 2.5 The hardware budget nobody has measured yet: host BAR1 size

A CPU view of vidmem consumes **host** BAR1 virtual space. Without Resizable BAR a GA106 exposes
**256 MiB** of BAR1, of which the host driver already uses some. The guest's `bar1-size` (256 MiB
default, `nvkvm.c:3251`) bounds how much it can map at once; the sum of live guest BAR1 mappings
must fit in the host's free BAR1. ⚠ Measure on the bench before sizing: `lspci -vv -s <gpu>` →
`Region 1 … [size=…]`, and `nvidia-smi -q | grep -A3 'BAR1 Memory Usage'`. If ReBAR is enabled the
budget is the whole card. If not, set the guest's `bar1-size` below the host's free BAR1 or accept
named misses past it. This is the one limit that makes *"passthrough for everything"* a
hardware question rather than a design one.

### 2.6 Memory type, and why it is not a correctness question

The driver maps framebuffer views write-combining (`nv-mmap.c:587-597`). A KVM memslot over a
`VM_PFNMAP` pfn gets the hypervisor's EPT memory type for non-RAM, which on VMX is **UC** unless a
non-coherent-DMA device is attached (`arch/x86/kvm/vmx/vmx.c`, `vmx_get_mt_mask`) — so guest stores
through the slot are uncached, not write-combined. **Both are >1000× cheaper than a VM exit + software
page walk**; the difference is bandwidth (hundreds of MB/s vs GB/s), to be measured, not a
correctness gap. `Backing::attainable_cache_policy` answers `None` for a device file for exactly
this reason: the policy is decided below us and cannot be read back.

### 2.7 BAR2 stays trapped — and why that is not a weakening

> ⊘⊘ **SUPERSEDED 2026-09-09, the same day, by §7.1 — and the reasoning below is where the
> mistake was.** *"BAR2 is the observation point for the guest's page-table writes"* is true
> **only of an observation-driven mirror**. The mirror that landed is **demand-driven** (a
> memslot is installed on the first-touch miss of a page and revalidated on the guest's own
> `MMU_INVALIDATE`), so nothing needs to watch the guest write its tables — the access **is**
> the notification — and BAR2 has no watchpoint role left. Owner: *"we can also make bar2
> untrapped"* → *"ok go"* (commit `f55984af`). BAR2 is now armed by `bar2-passthrough` on
> exactly BAR1's terms. The publish-trigger dependency named below is dissolved for the BAR
> apertures for the same reason: a first-touch miss is a trigger that cannot be missed.

BAR2 is the CPU-visible page-table/instance-block aperture: **it is the observation point** for the
guest's page-table writes, including the BAR1 page table the mirror in §2.3 depends on, and the
GR/CE VAS tables the publication plane depends on. The owner's publish-trigger ordering
(`publish_trigger_preference_ordering.md`) puts *"trap the PTE write"* second only to the exact GPU
boundary (TLB invalidate) — which the guest **does not fire** for most rows (w390: 25 of 74). Passing
BAR2 through therefore needs a replacement publish trigger first, and that is a separate owner
decision. Its traffic is control-plane (page-table and instance-block dwords), not tensor bytes.
⇒ Out of scope here **by name**, with the dependency stated.

---

## 3. What landed on `w393-bar-passthrough` — the crossing and the placement, end to end

All `cargo check` clean per crate; the proto round-trip tests pass (16/16). **No production path
reaches any of it**; every piece is a named verb behind a default-off arm.

| crate | change | file |
|---|---|---|
| `kayfabe-isolate` (pure) | `DeviceView { token, memory, offset, mmap_len }`; `RmBackend::export_device_view(memory, offset, len)` **defaulted to `NotExportableAsMemory`** so mocks/loopback refuse by name; `Worker::export_device_view` with R1 + the foreign-handle gate | `src/lib.rs` |
| `kayfabe-isolate-host` | `RmConnection::arm_cpu_view` — the first half of `map_cpu_windowed_on` (open fresh node + `0x4E` with an object **offset**) as its own verb; `map_cpu_windowed_on` now calls it (every existing caller unchanged, offset 0). `HostRmBackend::export_device_view` arms and mints via `ChildExports::mint_armed_node`. Wire: `Request::ExportDeviceView` (tag 25) / `Reply::DeviceView` (tag 12); child `serve_one` intercept (third descriptor-carrying reply); `ProxyRmBackend::call_for_device_view` adopting the fd as **`DescriptorKind::CharDevice`, established from the kernel** | `src/rm.rs`, `src/proto.rs`, `src/child.rs`, `src/isolate.rs` |
| `kayfabe-linux-raw` | `GuestWindow::place_device_view(offset, len, fd)` — `MAP_FIXED\|MAP_SHARED` of an armed node at file offset 0. `place` **still refuses** `Backing::DeviceFile` (its test stays green); the legitimate use is a named door | `src/window_unsafe.rs` |
| `kayfabe-vmm-qemu` | `WindowBacking::{Minted, DeviceView(fd)}`; `QemuMachine::install_device_window(gpa, len, fd)` — the foreign-slot install with the device view placed whole (no `SharedRam`, no `export_ram` for it) | `src/lib.rs` |
| QOM glue | `bar1-passthrough` property (default **off** = control). ON: `bar_is_unbacked_reservation(BAR1)` answers **yes** (truthful — the constructor is unchanged); every BAR1 access that still traps is counted `bar1_passthrough_misses` and printed by name (bounded 8); realize banner `nvkvm: BAR1-PASSTHROUGH arm=…`; teardown row printed unconditionally. The `window-size` whole-BAR shadow stays refused | `qemu/hw/misc/nvkvm/nvkvm.c` |
| `kayfabe-rm-ladder` | `--bar1-crossing` (§6): the bare-metal proof of the crossing and of a real vCPU store through a memslot over a device view | `src/bin/rmladder.rs` |

### 3.1 Why `export_backing(HostDeviceMemory)` still refuses

It asks for the card's pages **as memory** — a thing the VMM may place anywhere and `export_ram`
twice. That stays `NotExportableAsMemory`; the correction block on `HostRmBackend::export_backing`
(`rm.rs:5233-5262`) already named the three "doors" as one policy argument (answered by the armed
node), one hardware fact that is moot (dma-buf), and one refusal of our own (`place`). The new verb
crosses a **mapping context**, and the placement is a **separate door**. Nothing was relaxed in place.

### 3.2 Decision (b), stated for the ruling

What crosses is a `/dev/nvidia<N>` descriptor with an RM escape handler behind it. The property that
keeps `isolate_vmm_fd_crossing.md` §12 honest is **structural on the VMM side**: the VMM issues no
escape on it — only `mmap` — and `secInfo.privLevel` is recomputed **per escape** from the caller
(`ogkm-580: escape.c:304`), so a process that never escapes gains nothing. The VMM closes the
descriptor the moment the `mmap` returns (the VMA keeps the `struct file`). `nvkvm-pv` ships exactly
this shape (`src/qemu/nvkvm_isolate_handlers.c:3618` → `nvkvm_mmap_host.c:1269` + `:1203`).
⚠ **Whether the VMM may hold such a descriptor at all, even transiently, is the owner's call.** The
branch makes the mechanism real and checkable; it does not switch it on.

### 3.3 Readings that became testable

- `nvidia_mmap_helper`'s framebuffer arm gates on `mmap_context->valid`, `vm_pgoff == 0`, GPU state
  and `safe_to_mmap`, sets `VM_IO | VM_PFNMAP | VM_DONTEXPAND`, and **names no calling process**
  (`ogkm-580: kernel-open/nvidia/nv-mmap.c:505-641`). Nothing clears `valid` after a successful
  `mmap`; it is freed at file close (`nv.c:1054`). ⇒ an armed node can cross a process boundary and be
  mapped there. **Reading → measured by §6 leg A.**
- A KVM memslot over a `VM_PFNMAP` VMA serves guest accesses natively (VFIO's shape; `nvkvm-pv`
  installs `KVM_SET_USER_MEMORY_REGION` over the device `mmap` at `nvkvm_mmap_host.c:1203`).
  **Reading → measured by §6 leg B.**

### 3.4 The arm is a census, not a passthrough, on this build

With `bar1-passthrough=on` and no mirror, **every** BAR1 access is a named miss. That is deliberate:
the miss log (offset, size, direction, uncapped total) is the census of runs a mirror must cover, and
the acceptance test's criterion 1 is *this counter reading zero* once it does.

---

## 4. What remains — in order, with the ruling each needs

1. **Owner ruling: decision (b) scope** (§3.2). Without it nothing below may be wired to production.
2. **Bare-metal measurement** (§6) — turns the two readings into facts; also measures the host BAR1
   budget (§2.5) and the memslot memory type (§2.6).
3. **The lease** — a GPGA frame → host vidmem object table ("memory is owned globally, procs lease
   it"), leased at first naming by any path, 2 MiB granule, freed when no page table names it.
   This is where `SparseFb` stops holding bytes for leased frames; trapped paths (BAR0 window, BAR2,
   the walker, the GSP emulation) read/write leased frames through a windowed CPU view.
   ⚠ `copy_placement_policy.md` §2.2: the moment a frame is vidmem, any bulk fill must be a CE copy,
   not a CPU memcpy across the BAR.
4. **The mirror** — walk `bar1PdeBase` at PTE publication (BAR2/BAR0-window trap on a page the walker
   has traversed) or at first touch (the miss is the evidence), coalesce contiguous runs onto one
   leased object, `export_device_view(object, frame_offset, run_len)` → `install_device_window`;
   tear down on PTE overwrite **before** the frame can be re-leased. Slot budget:
   `OUR_SLOT_BUDGET = 64` (`slots.rs`) vs the observed BAR1 population (4 UVM channel pages + RM's
   USERD + a raw client's operands) — raise if the census says so.
5. **Sysmem-aperture BAR1 PTEs** — memslot aliasing guest RAM (§2.4), engine side via the pin.
6. **Retire the join** for leased frames: `join_fb_leaf` degenerates to `map_gpu_va` of the lease;
   release/carry/revoke have nothing to carry. ⊘ Not touched on this branch — the owner is mid-test
   on `join_one_fb_leaf`'s extent arms.
7. ~~**BAR2** — only after a replacement publish trigger (§2.7); separate ruling.~~ ⊘ Done
   in §7: the demand-driven mirror needs no publish trigger, and BAR2 has its own arm.

---

## 5. What the arm does and does not measure on a boot (for whoever boots it first)

`-device nvkvm-gpu,…,bar1-passthrough=on` (property; the boot's own census prints the arm):
- `nvkvm: BAR1-PASSTHROUGH arm=on ⇒ …` at realize; `nvkvm: BAR1-PASSTHROUGH arm=on misses=N …` at
  teardown, beside the existing `BAR1 access log` and `GP_PUT` rows.
- ⚠ **On this build the miss total equals `bar1_touches`** — there is no mirror. A boot that prints
  `misses=0` under `arm=on` has not touched BAR1, not passed it through; read it beside the
  trap-status row and `bar1_touches`.
- The archive's `bar1_reads`/`bar1_writes` counters are unchanged and still count every trapped access.

---

## 6. The bare-metal probe — `kayfabe-rm-ladder --bar1-crossing`

Two legs, each a named PASS/FAIL, run as the raw client on the host (no guest, no QEMU):

- **Leg A — the crossing.** Parent allocs a 64 KiB vidmem object, arms **two** fresh nodes on it
  (`export_device_view` ×2), sends node A over `SCM_RIGHTS` to a spawned child (`--bar1-crossing-child`,
  socketpair as stdin) and **closes its own copy**; the child kind-checks the fd against the kernel,
  `mmap`s it at offset 0, stores 64 pattern words, fences, drops everything. The parent reads the
  words back through node B. Negative control: the parent zeroes and reads back through B first.
  PASS ⇔ 64/64 agree ⇒ **an armed node crosses a process boundary and the two views are one memory.**
- **Leg B — the guest store.** (Skipped **by name** without `/dev/kvm`.) A third node C is placed
  whole in a `GuestWindow` via `place_device_view`, installed as KVM memslot 1 at `0x1000_0000`; a
  flat-protected-mode vCPU runs `mov [data], PATTERN; mov eax,[data]; mov [ebx],eax; hlt` with `ebx`
  = an address no slot covers. PASS ⇔ the **only** exit is the signal store at `ebx` carrying
  `PATTERN`, **and** word 0 reads `PATTERN` through view B afterwards ⇒ criteria 1 and 3, measured.
  An exit at `data` is the KVM-over-`VM_PFNMAP` question answered **no** and is printed as such.
- ⊘ **Not measured:** that the **engine** reads the bytes (criterion 2). The GPU view is
  `map_gpu_va` over the same object (R25/R30 exercise it); a CE readback in this rung is the next
  step.

Build and run (the bench recipe, `scripts/bench/r33_hook_ce_client.sh:53`):
```
cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder -p kayfabe-isolate-host
scp target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder <box>:/tmp/ && ssh <box> '/tmp/kayfabe-rm-ladder --gpu N --bar1-crossing'
```

### 6.4 Result

_(filled in by the run; if this line is still here, the readings in §3.3 are readings.)_

---

## 7. ★★★★★ THE DEMAND-DRIVEN MIRROR — increment 2, BAR1 *and* BAR2 (2026-09-09)

**Owner direction (commit `f55984af`):** build the first-touch mirror; it removes BAR2's
watchpoint role, so untrap BAR2 too. **Acceptance:** `misses` falls from ~87,736 to
approximately the number of DISTINCT FRAMES touched; the client still prints
`W392D_OUTCOME=(P)`, `THREADS 4 of 4`, `MEAN_FALSIFIER=PASS`; the archive's `bar1_reads` /
`bar1_writes` stop moving for covered frames; BAR2 likewise with its own arm and census.

### 7.1 The mechanism, in one paragraph — the mirror IS the guest's BAR TLB

A trapped BAR1/BAR2 access is served exactly as before (page walk through the guest's own
BAR page table → the store). **Then**, with the plane lock released, the shell treats the
access as a **TLB miss** and fills: it re-walks the page (`RegPlane::window_page_backing`,
`plane.rs`), asks the store **what memory backs that frame** (`FbStore::page_backing`,
`fbwin.rs`) — materialising the page into the **page arena** if it is not resident or is a
heap page — and installs a **4 KiB KVM memslot** over *that memory* at the page's
guest-physical address (`QemuMachine::install_file_window`, `kayfabe-vmm-qemu/src/lib.rs`).
Every later access to the page is exit-free and lands in the very bytes the walker, the BAR0
window, the GSP emulation and the join all read: **one memory, two names, nothing copied**.
The flush is the guest's own `MMU_INVALIDATE` trigger (and a BAR2 root re-publication):
`BarMirror::revalidate` re-walks every live entry and drops only those whose translation or
backing changed (`crates/kayfabe-qemu-raw/src/barmirror.rs`).

### 7.2 Where the bytes live now — three page kinds, one store

| the store's page is | a memslot over it names | who else maps it |
|---|---|---|
| **arena page** (`kayfabe_linux_raw::SharedPageArena`, `arena_unsafe.rs`) — a 4 KiB index of one sealed 1 GiB sparse `memfd`, mapped once for the store | `(arena memfd, index × 4096)` | nobody but the guest (through the slot) and the store |
| **joined leaf** (`MappedFb` over the join's `memfd`, `shim.rs`) | `(join memfd, join offset + page offset)` via `JoinRegistry` | the isolate (`OS_DESCRIPTOR` → the engine) |
| **heap page** (`Box<[u8;4096]>`) — only a page created before the arena was installed, or while the arena was exhausted | — (refused by name, `HEAP-PAGE`) | nobody; served by the trap |

`SparseFb::pages` is now `HashMap<frame, FbPage>` with `FbPage::{Heap, Arena}`; every
new page comes from the arena once `install_page_arena` ran (at memory-plane attach, only
when an arm is on — the control's store is byte-for-byte the old one).

### 7.3 ★★★ The invariant, and the two places it is enforced

> **A memslot over a framebuffer range exists only while the store serves that range from
> the very pages the slot names.**

1. **Before the store moves a range's bytes** the plane calls `FbMirrorPort::quiesce`
   (`plane.rs`: `join_fb`, `release_fb_join`, `release_fb_join_carrying_bytes`,
   `device_reset`) with **no ranked lock held**; the mirror retires every slot over the
   range and refuses fills there until `resume`. This is the ordering the join's own doc
   demanded (*"mapping after execution seems racy"*): the second name is taken away
   **before** the copy, so a guest store cannot land in a page that is about to be dropped.
2. **After installing a slot** the fill re-resolves the page under the plane lock and keeps
   the slot only if translation and backing are what it installed over, and its *pending*
   ticket survived (a quiesce or a competing fill cancels it). A slot that lost the race is
   removed before anyone can hit it (`RACED-AND-DROPPED`, counted).

Every memslot ioctl runs outside the plane's `RankedMutex` (R1); the mirror's own table is
a plain mutex never held across a syscall.

### 7.4 The arms, the censuses, and how to read them

- **QOM:** `bar1-passthrough=on` / `bar2-passthrough=on` (`NVKVM_DEV_EXTRA=bar1-passthrough=on,bar2-passthrough=on`
  through `boot_nvkvm.sh`). Each flips one answer — `bar_is_unbacked_reservation` for that
  row — and turns every trapped access into a NAMED miss (`BAR1-PASSTHROUGH MISS #n`,
  `BAR2-PASSTHROUGH MISS #n`, bounded live, uncapped total). **The archive learns the arm by
  asking the hypervisor that same question** (`QemuMachine::bar_is_unbacked_reservation`), so
  there is no second flag that could disagree.
- **Archive banners at attach:** `BAR-MIRROR bar1: ARMED …` / `OFF (the control)` /
  `NOT BUILT` (no `host-isolates` feature). ⚠ A boot whose C says `arm=on` and whose archive
  says `NOT BUILT` is the census-only build of §3.4 — every access traps.
- **Archive census (END OF RUN, DETACH, and every 512 fills):**
  `BAR-MIRROR bar1 AT …: arm=on fills=F distinct_pages=P distinct_frames=D` and
  `BAR-MIRROR MECHANISM AT …: slots live/peak, revalidate[runs kept removed],
  quiesce[calls removed], retire_all[…], arena[…], refused=[NAME=n …]`.
- **The C's one-instant line:** `BAR1 COUNTERS (one instant): entries=… touches=… (reads=…
  writes=…) misses=… touches-minus-misses=…` — the three counts of one event that the
  2026-09-09 census could not reconcile (87,740 vs 87,736) are now printed together; under
  `arm=on` the delta must be 0 within one report, and the report is printed twice (exit
  notifier and device exit).
- **Reading rule:** `misses ≈ fills + Σ refused`. `fills ≈ distinct_pages + slots removed by
  revalidation/quiesce that were re-touched`. The acceptance ratio is `misses / distinct_frames`.

### 7.5 What this increment is NOT

- It is **`HOST_VISIBLE` on the memory the store already uses** — sysmem (arena / join
  `memfd`), the same memory the engine reads for joined leaves. It is **not yet
  `DEVICE_LOCAL`**: that needs the lease (§4.3) so a leaf's backing is a vidmem object; the
  mirror is backing-agnostic (`FbPageBacking` names a token + offset) and the crossed device
  view of §3 plugs in as a third page kind when that lands.
- A read-only PTE gets a **read-only slot** (writes still trap and are refused by name);
  a sysmem-aperture PTE is still `BAR1_FOREIGN_APERTURE` (a named `TRANSLATION-REFUSED`
  fill refusal).
- Slot budget: `MIRROR_SLOT_BUDGET = 4096` on top of `OUR_SLOT_BUDGET = 64`
  (`slots.rs`, taken only when the kernel ceiling allows; the attach banner states the
  range). Past it the fill refuses by the allocator's own name and the page traps.

### 7.6 Result — MEASURED 2026-09-09, box 50389992 (RTX 3060 GA106, host 580.159.04 open,
guest 580.159.04 open on 6.8.0-138), ONE binary `kayfabe-rev:55ff7b56`, three arms, the
w392d mean client (`--uvm-mean --mean-falsify`) under the legacy arming set

| arm | BAR1 misses (C) | BAR1 fills / distinct pages / distinct frames | BAR2 misses (C) | BAR2 fills / pages / frames | client | host Xid |
|---|---|---|---|---|---|---|
| **off** (control) | **88,193** (8,256 r / 79,921 w) | mirror OFF | 0 (arm off) | mirror OFF | `W392D_OUTCOME=(P)` `THREADS 4 of 4` `MEAN_FALSIFIER=PASS` | 0 |
| **bar1** | **91** (2 r / 89 w) | 84 / 36 / 42 | 0 (arm off) | mirror OFF | `(P)` `4 of 4` `PASS` | 0 |
| **both** | **90** (1 r / 89 w) | 84 / 39 / 43 | **384** | 196 / 92 / 101 | `(P)` `4 of 4` `PASS` | 0 |

★★★★★ **Criterion 1: BAR1 misses fell 88,193 → 91 (969×), to ~2.2 per distinct frame** (42
frames; the surplus is the 81 slots a revalidation retired and the guest re-touched — the
guest re-maps the same BAR1 pages to new frames every round, `distinct_frames > distinct_pages`).
**Criterion 2: the client passes on every arm**, all four rows and 4 of 4 threads, Xid 0.
**Criterion 3: the archive's `bar1_reads`/`bar1_writes` audit read 2/89 under the arm** (vs
8,256/79,952 on the control) — the counters only move on fills. **Criterion 4: BAR2 has its
own arm and census**: 384 misses for 196 fills over 92 pages.

**The mechanism's own numbers (arm=both):** slots peak 125; `revalidate[runs=692 kept=53,189
removed=277]` — one run per guest `MMU_INVALIDATE` trigger, re-walking every live entry and
retiring only the changed ones (never a wholesale flush); `quiesce[calls=67 removed=3]` — 67
join installs/releases passed through the plane's port, retiring 3 slots that sat over frames
about to move; arena 1,713 pages live (7 MiB), 83 recycled; **no `QUIESCE WAITED`**, no
`HEAP-PAGE`, no `SLOT-BUDGET`, no `JOIN-GONE`.

⊘ **`refused=[ALREADY-COVERED=192 RACED-AND-DROPPED=2]` on BAR2 (and 4 / 3 on BAR1) are not
defects, and reading them as "192 misses the mirror could not fill" would be wrong.** The
printed rows are pairs at `+0` and `+4` of one page, microseconds apart: the guest's
**8-byte** PTE store arrives as ONE MMIO exit that QEMU dispatches as **two 4-byte
callbacks** (`nvkvm_bar2_ops` declares no `.impl.max_access_size`, which defaults to 4). The
first half fills; the second half of the *same exit* finds the slot already there. It costs
nothing beyond the one exit that was already taken. ⇒ For BAR2, `misses ≈ 2 × fills` is the
split, and the *distinct-page* number (92) is the honest denominator.

★★★★★ **THE 4-ACCESS DISCREPANCY IS ANSWERED, AND IT WAS NEVER FOUR ACCESSES.** The control
arm's one-instant line read `entries=88208 touches=88193 (reads=8256 writes=79921)` — three
counters bumped three lines apart in the same two callbacks, **three different totals**,
while the archive's atomic counters said `8256 / 79952`. Every region of this device is
`memory_region_enable_lockless_io` (`nvkvm.c`, `nvkvm_region_init_io`), so the BAR callbacks
run **concurrently on every vCPU with no BQL**, and every `x++` in them was a plain
load-add-store: **lost increments**. The 2026-09-09 `87,740 vs 87,736` was the same race
(a smaller one — that boot had fewer vCPUs racing). Fixed in the same commit: every counter
in the handlers is `qatomic_*`, and the two bounded logs (`bar1_log`, `gp_put_pages`) —
whose `used++` under a `>=` check could index **one past the end** under the same race —
sit under a `QemuSpin`. ⚠ It also falsifies a standing comment in the archive
(`kayfabe_shim_regs_write`: *"arrives here with the QEMU BQL held"*): it does not. The
archive's own counters are atomics and its plane is behind a ranked mutex, so nothing on
the Rust side was wrong; the mirror's table was built for concurrent fills from the start
(`pending` tickets), and `PENDING-ELSEWHERE=0` on every arm says two vCPUs never raced one
page.

⊘ **What this measures and what it does not.** One raw client, one boot per arm (n=1 per arm
at this revision; a second ladder at the counter-fix revision is in §7.7). It says nothing
yet about the LLM or about throughput: the client's rounds are small, so the *cost* of a
fill (two `mmap`s + one `KVM_SET_USER_MEMORY_REGION`) and of a revalidation (692 runs ×
~76 walks) is not resolved by its wall time; the throughput question is the next boot's.
`DEVICE_LOCAL` is still §4.3's lease — this is `HOST_VISIBLE` over the memory the engine
already reads.

# The host execution plane — decisions, 2026-07-29

> **Status: DECIDED (owner, 2026-07-29). This supersedes parts of
> `l2_qemu_adapter.md` — see §1.** Written from a working session; every claim below is
> either measured against the tree, cited to the C artifact, or explicitly marked as an
> assumption.

## 0. ★★★ The finding that prompted this

**The host execution plane does not exist.** Measured at `f0053ef`: the only implementors
of `IsolateFactory`, `Isolate`, `RmBackend` and `Present` are in `kayfabe-mocks`. Nothing
spawns a host process; nothing issues a real RM ioctl. `kayfabe-vmm-kvm` is real. The
entire *host* side is a double.

That matters more than any other open item for one reason:

★★ **The isolate's defining hazard is unmockable as currently modelled.** RM serialises
**all** ioctls per client, waits **UNINTERRUPTIBLY**, and one wedged verb puts every
isolate sharing that client into D-state (`rm_concurrency_semantics`, C-measured). The
whole L1 design — **R1** (no blocking call under a ranked lock), **F1** (every poll
provably bounded), **I-NOAMP** (one thread's stall must not amplify) — exists to survive
exactly that, and the suite has **never seen it**, because a mock returns promptly by
construction.

We have direct evidence this bites. `07da582` found a cross-GPU handle accepted by the
foreign-handle gate and caught **only by the mock** — `HostHandle`'s own docs state a real
host does *not* catch it, because RM mints every client's handles from one
`RS_CLIENT_HANDLE_BASE`. One known divergence, found by accident. The open question is how
many remain unfound.

## 1. ★★★ DECISION — the QEMU memory plane owns its own memslots

**Supersedes `l2_qemu_adapter.md` §5.4 (and dissolves its conflict with §4.3).**

Do **not** express the guest-visible memory map through QEMU's `MemoryRegion` tree.
Instead:

- QEMU **reserves the GPA window** (the BAR range) and does not back it;
- we install our own memslot(s) via `KVM_SET_USER_MEMORY_REGION` and populate by `mmap`;
- **passthrough** pages are ordinary RW memslot;
- **read-native** pages use `KVM_MEM_READONLY` — reads served from RAM at native speed,
  writes exit to us, which *is* the read-native semantic;
- **observe-everything** pages have no slot, so accesses MMIO-exit.

### 1.1 Why — three arguments, in order of weight

1. ★★ **It dissolves the §4.3/§5.4 contradiction rather than working around it.** §4.3
   forbids any topology mutation outside realize/unrealize (we never call `bql_lock`);
   §5.4 spells the coarse tier and the read-native overlay as *added subregions*, which are
   topology transactions — while `Vmm::map_read_native` is a **runtime** method. They
   cannot both hold. Under this decision we never touch QEMU's tree after realize, so
   there is nothing to reconcile. (`f0053ef` shipped a latch-and-claim workaround for the
   old shape; that machinery becomes unnecessary.)
2. ★★ **It is the right performance model, and QEMU's API defaults to the wrong one.**
   QEMU's `MemoryRegion` API is built around *"give me a handler and I will trap"*. For a
   multi-GiB BAR we want the inverse: passthrough at native speed, trapping only the pages
   we must observe. **[measured, C]** `mode2_baremetal_32` — Mode-2 LLM 49.9 t/s vs host
   47.5 t/s, i.e. zero overhead bare-metal.
3. ★ **It makes the VMM seam THINNER, which helps a Cloud Hypervisor port** — the opposite
   of the intuition. If we own the memslots, a VMM's entire job is *"reserve this GPA
   window and stay out of it"*, which is small and portable. Expressing everything through
   QEMU's memory API couples us to **QEMU's abstraction**, and a CH port would redo the
   memory plane from scratch. Owning KVM means depending on KVM, which both sit on.

### 1.2 ★ This is what the C did — cited, not assumed

- Raw `KVM_SET_USER_MEMORY_REGION` on the VM fd — `C: src/qemu/nvkvm_mmap_host.c:482`,
  with a **centralised slot allocator** in that file.
- `nvkvm_sparse_init()` installs the window's memslot **once**; thereafter
  *"zero per-mmap KVM"* — `C: src/qemu/nvkvm_isolate_handlers.c:1792`.
- The two-tier scheme is recorded as `bar1_memslot_perf`: memslot-back host-written
  read-mostly pages, trap observe-write pages.

### 1.3 ★★ The pointer question DISSOLVES — no new unsafe relaxation

An earlier reading held that Q2 owed `kayfabe-linux-raw` a relaxation, because
`GuestWindow` has no base-address accessor and §5.1 assumed we would "hand QEMU the
pointer". **That was an artifact of §5.4, not a requirement.** Handing QEMU a pointer
(`memory_region_init_ram_ptr`) would force the base across a *crate* boundary into
`kayfabe-qemu-raw`'s unsafe file, which the host-pointer gate forbids. Under this
decision we call KVM ourselves, so the unwrap happens inside `kvm_unsafe.rs` on a safe
`&GuestWindow` and **the pointer never leaves the crate that owns it**. The gate holds as
designed; the ratchet stays at 37.

### 1.4 Named risks — decided WITH these on the record

- ★ **QEMU must not place a conflicting slot in the window.** The C hit exactly this and
  records the fix as proven — `C: src/qemu/virtio_nvgpu_pci.c:32` refers to a collision
  with *"the window's own raw `KVM_SET_USER_MEMORY_REGION` slot"*. **Read that fix before
  implementing; do not re-derive it.**
- ★ **This is more fragile than the `MemoryRegion` API.** We depend on QEMU tolerating
  slots it does not own — behaviour it does not promise. Accepted deliberately: the
  performance and portability wins are judged worth it. If a future QEMU breaks it, the
  failure should be loud at realize, not silent at runtime.
- **[unverified]** Whether QEMU's KVM listener ever recomputes and clobbers foreign slots
  (BAR remap by guest firmware, hotplug). The C survived it; **find out why** rather than
  assuming. → **RESOLVED, see §1.5.**
- **[unverified]** Whether the C needed a QEMU patch for any of this. It ran on 9.2.0 from
  `C: scripts/build_qemu.sh:10`. → **RESOLVED: no patch. See §1.5.**

### 1.5 ★★★ RESOLVED against QEMU 9.2.0 source, 2026-07-29 — safe, and here is *why*

**The reservation object is a pure MMIO `MemoryRegion`.** `memory_region_init_io` +
`pci_register_bar` as a 64-bit prefetchable BAR — `C: src/qemu/virtio_nvgpu_pci.c:108-114`,
size `128 GiB` at `:41-42`, with stub ops that *"are never reached normally"* (`:50-66`). Not
a container, not a reservation API (there is none), not nothing.

**It is structurally invisible to KVM, which is the whole answer.**
`qemu: system/memory.c:1568-1579` — `memory_region_init_io` never sets `mr->ram`. So
`memory_region_is_ram()` (`qemu: include/exec/memory.h:1690-1693`, literally `return mr->ram;`)
is false, and `qemu: accel/kvm/kvm-all.c:1449-1457` takes an **unconditional early return**
for a *writable non-RAM* region — **before** `kvm_alloc_slot`, before
`kvm_lookup_matching_slot`, before any ioctl.
⇒ **QEMU never creates, deletes, or even looks up a memslot for our BAR range**, in either
direction. A BAR remap (`qemu: hw/pci/pci.c:1535-1575`, `del_subregion` +
`add_subregion_overlap`) feeds `kvm_region_del`/`kvm_region_add` which both land on that same
early return, so a remap produces **exactly zero KVM slot operations**. Hotplug and
`has_power` route through the same `pci_update_mappings`. Every whole-array sweep in
`kvm-all.c` is bounded by QEMU's own `kml->slots[]` and never enumerates KVM's table — QEMU
has no read-back ioctl. **QEMU is structurally incapable of clobbering a slot it did not
create, except by reusing its id.**

**No QEMU patch is needed.** `C: scripts/build_qemu.sh` patches exactly two upstream files —
`hw/misc/meson.build` (`:120-173`) and `hw/virtio/virtio.c` (`:175-211`, the
`virtio_device_names[]` assert). `accel/kvm/` and `system/` are **stock 9.2.0**. That makes
this a *source-level* guarantee, not an empirical one, and it should hold on Cloud Hypervisor
too, which likewise tracks only its own slots.

#### ★★★ But the C has a LATENT BUG here, and "it survived" is the wrong lesson

`C: src/qemu/nvkvm_mmap_host.c:189-193` caches the window base **once** and never re-resolves
it; `window_base_get` has no other caller and there is **no BAR-change hook anywhere in
`src/qemu/`**. So if the guest moves BAR2 after the slot is installed, QEMU's MMIO region
follows and **our memslot does not** — the window keeps working at the *old* GPA (now
unreserved, and available to another BAR) while the *new* GPA reads zeros. Silent.

It never fired for three reasons, all evidenced: the install is lazy, on the first GPU mmap
(`:241-243`), long after PCI resource assignment; the transient unmap during Linux's BAR
sizing **is** properly guarded (`qemu: hw/pci/pci.c:1494-1496` →
`C: virtio_nvgpu_pci.c:74-75` → `C: nvkvm_mmap_host.c:196-204`, real code not just a comment);
and Linux honours firmware BAR assignment absent a conflict or `pci=realloc`.
⇒ **The mechanism is sound; the C's implementation of it carries an unguarded assumption.**
**We must close it:** latch the base at first install and *assert it unchanged* on every
resolve, failing **loudly** — which is what §1.4's second bullet already demands of us.

#### ★★ Correction to §1.4's first bullet — comment ≠ measurement

`C: virtio_nvgpu_pci.c:30-33` states the collision was *"proven by the earlier probe"*. The
probe writeup says only *"the **likely cause** is a memslot conflict"*
(`C: docs/design/gpa_window_pci_bar.md:68-74`). What is **evidenced** is that adding a
`memory_region_init_ram_ptr` BAR regressed `cuInit` and removing it restored matmul. The
*mechanism* was never root-caused, and both candidate mechanisms look implausible: a slot-id
collision needs 64 simultaneously live AS-0 slots (a Mode-2 VM uses well under 16), and a GPA
overlap would have `abort()`ed QEMU loudly (`qemu: kvm-all.c:1516-1521, 1537-1541`) — the
probe explicitly reports *no* QEMU/KVM error. **This does not endanger §1**, which proposes
the shape the C shipped and measured green; it means the *rejected alternative's* failure is
unexplained. Third instance of the comment-vs-measurement trap.

#### ★ Slot ids: allocate TOP-DOWN, not from a hardcoded base

The C uses `NVKVM_KVM_SLOT_BASE 64` / `COUNT 448` (`C: nvkvm_mmap_host.c:390-435`) on the
convention that *"below the base is reserved for QEMU's static regions"* — **enforced by
nothing**. QEMU allocates densely from 0 (`qemu: kvm-all.c:250-262`, `slots[i].slot = i` at
`:208-210`), starting at 16 slots and doubling. So disjointness holds by arithmetic *for this
device set*, and breaks under virtio-mem, memory hotplug, or several VFIO devices with RAM
BARs. **Allocate our range from the top of `KVM_CAP_NR_MEMSLOTS` instead** — QEMU grows
upward and `abort()`s before wrapping, so collision would require exhausting the whole space.

#### ★ §1.5a The window is KVM-visible but QEMU-FlatView-OPAQUE — a real constraint

The shadowing is KVM-only. Anything reaching the window through QEMU's `FlatView`
(`address_space_rw`, `pci_dma_read/write`, `memory_region_find`) hits the stub ops and gets
**zeros / silent discard**, not our memory. Mode 1 never did this; we will.
⇒ **`Vmm::gpa_read`/`gpa_write` must NOT route window GPAs through QEMU** — they must use our
own HVA arithmetic — and `GuestRamMap` will (correctly) classify the window as `Device`.

#### ★★ Scope: §1's tiering is NEW CONSTRUCTION, not a port

The measured artifact is the **Mode-1** virtio-nvgpu window. **Mode 2 has never had a
memslot**: `C: nvkvm_gpu_emul.c:9743-9779` makes BAR0/BAR1/BAR3 all trapping MMIO, and
`KVM_SET_USER_MEMORY_REGION` appears **nowhere** in that file; `bar1_memslot_perf` is a
roadmap note, not code. So the C is precedent for **reservation + one RW slot** and for
nothing else. `KVM_MEM_READONLY` read-native and the mixed passthrough/observe taxonomy must
earn their own green gate.

### 1.6 ★★★ BUILT, 2026-07-29 (stage Q2–Q5) — and the five things the build FOUND

§1 is no longer a decision; it is the memory plane. What exists: a memslot seam with a
**real** implementation (`kayfabe_vmm_qemu::slots::KvmSlotPlane` over
`kayfabe_linux_raw::KvmVm`) and a mock that refuses where the kernel refuses; a descending
allocator; the three tiers; the BAR latch in three parts; and task #97's refusal. 904 tests,
15/15 gates, and every new guard **watched failing** before it was trusted.

**What is NOT built, stated first:** the C QOM shim. `kayfabe-qemu-raw` is still empty,
because it needs a hypervisor source tree to compile against and this machine has none.
Everything below the seam is real and runs against a real kernel; everything above it is
still a double. That is the honest boundary of this stage.

#### ★★★ Finding 1 — §1 does not merely fix §4.3/§5.4, it DELETES a constraint

`f0053ef` built the coarse tier **realize-only**, because publishing a region was a topology
transaction and §4.3 confines those to realize/unrealize. That made `Vmm::map_read_native` a
method which could only *claim* an overlay realize had already created, with four refusals
attached to the impossibility, and it made `TOPOLOGY_AFTER_REALIZE` a design constraint
dressed as a lifecycle error.

Under §1 **none of that exists**: installing a window is a call to the kernel, legal at any
time, and `map_read_native` creates one exactly as the sibling backend does. The refusal lost
its subject. What replaced it is `MEMORY_PLANE_AFTER_UNREALIZE`, which is a real and reachable
lifecycle error. ⇒ The decision's value was under-stated in §1.1: it is not a third argument
about performance and portability, it is the removal of a restriction that had propagated
into the port's semantics.

#### ★★★ Finding 2 — the opacity (§1.5a) needs a LAYERING, not a rule

§1.5a says `gpa_read`/`gpa_write` must not route window addresses through the hypervisor. A
rule is not enough, because the failure is **silent and successful**: the reservation BAR's
stub ops return zeros and discard writes, so a bypassed window lookup does not error — it
returns what freshly-zeroed guest memory would.

So the built shape is two layers that disagree on purpose. Our own window map answers first,
by our own offset arithmetic, with no hypervisor call at all. The region map keeps declaring
the window's BAR **`Device`** — which is *correct*, because that is what the range is through
the hypervisor — so a bypass falls into `NonRamGpa` rather than into a memcpy of zeros. A
bite-check confirms it: neutering the window lookup turns the memory-plane suite red rather
than green-and-wrong.

★ **One visible consequence, recorded rather than smoothed over.** A range straddling the end
of a window reports **its own start**, where the KVM backend reports the **boundary** byte,
because our window is not a region-map region at all. Both are correct; the property that
matters — refused as a unit, not one byte copied — holds in both, and is asserted directly.

#### ★★ Finding 3 — the C's VM-descriptor mechanism ports, and needs one arm the C lacks

`C: src/qemu/virtio_nvgpu.c:1114-1141` finds the hypervisor's VM descriptor by scanning
`/proc/self/fd` for the symlink target `anon_inode:kvm-vm`, because the header that would
expose the handle is target-only. That mechanism ports unchanged and is
hypervisor-**agnostic**, which is the property §1.1 argument 3 wants from this seam.

Two things the C does not have:
* the descriptor is an anonymous inode, so `/proc/self/fd/N` cannot be **re-opened** —
  measured `ENXIO` on this host — and must be duplicated. That is this build's only new
  `unsafe` block (ratchet 49 → 50, itemised in `ci.yml`), and its race is closed by
  re-reading the link of the descriptor we now **own**, so losing it is a refusal and never a
  wrong machine;
* the C takes the **first** match and stops. With two machines in one process there is no way
  to tell which one the device is in, and installing into the wrong one **succeeds silently**.
  Ambiguity is a loud refusal here, distinct from "no machine at all" — one is a fact about
  the invocation, the other a configuration fact, and they send an operator to different
  places. All three arms are driven for real.

#### ★★★ Finding 4 — the real-kernel differential caught the mock lying TWICE, on its first run

The mock slot plane is what keeps the tiering assertions runnable where `/dev/kvm` is absent.
Its first draft modelled the kernel's `EINVAL`/`EEXIST` and stopped there. Running the
identical scenario list against a real kernel found **two divergences immediately**:

| request | mock said | the real path says |
|---|---|---|
| a misaligned memslot **length** | `EINVAL` | `RawError::Misaligned` — refused **before any syscall** |
| a **zero-length** memslot | `EINVAL` | `RawError::ZeroLength` — likewise |

Both are pre-syscall refusals by the window's own bounded accessor, which the kernel therefore
never sees. A mock reporting the kernel's answer for a condition the kernel never reaches
teaches every test above it to expect the wrong variant. ⇒ **`07da582` was not a one-off.**
Two more divergences, in the first double built after it, found by running the double against
the real thing rather than by reading it.

#### ★★ Finding 5 — a bite-check found a SECOND evaluation site for the tier rule

Fourteen of fifteen new guards went red when neutered. The fifteenth — *"the observe tier
installs no slot"* — **survived**: the install path filtered on `s.tier != Tier::Observe`
while `Tier::readonly_slot()` was the stated rule, so flipping the rule changed nothing. The
same decay `classify.rs` is a whole module to prevent, one crate over. Fixed by making
`readonly_slot()` the only site that decides; the guard bites now.

#### ★★★ §1.6's stated absence is CLOSED, 2026-07-30 — the C QOM shim exists and was run

§1.6 opens with *"What is NOT built, stated first: the C QOM shim. `kayfabe-qemu-raw` is still
empty, because it needs a hypervisor source tree to compile against and this machine has
none."* It has one now. The shim is built, and the memory plane above it has been driven from
inside a real hypervisor process — full account in `l2_qemu_adapter.md` §12a.

The one number worth repeating here, because it is §1 restated as a measurement: with a 64 MiB
reservation live inside QEMU 10.2.4, the device reports **`kernel slots live=1 installs=1,
regions the hypervisor backs=0`**. The hypervisor reserved the guest-physical range with a
pure-MMIO base-address register and backed nothing; the accelerator slot over that range is
ours, installed by our own code through the kernel's own ioctl, in the machine the hypervisor
created. §1.5's early-return argument held in practice, on two releases.

★ §1.5's *"allocate TOP-DOWN"* note and the BAR-latch findings were exercised end to end: the
shim carries both the **preventer** (a configuration-space write override that asks before
letting a base-address-register write through) and the **detector** (a re-read afterwards) —
the two halves the C artifact has neither of.

#### ★ What #97's own argument turned out to be

§8.5's reasoning is **void**, and the correction is worth keeping because it inverts which
memory is at risk. The balloon skips our reservation *trivially* now — it walks only the
machine's own RAM blocks and ours is not one. The live hazard is **guest RAM exported to
isolates**: a shared `memfd` that the discard helper punches with `FALLOC_FL_PUNCH_HOLE`,
destroying the file's pages and so reaching **every** mapping — the hypervisor's, the
accelerator's second-stage tables, and the isolate's. The `-EBUSY` arm refuses realize
**naming the conflicting device**, because the two cannot coexist and an operator told only
*"a requirer is present"* has to bisect their own command line.

## 2. ★★ DECISION — model RM's semantics in the mock, deterministically

**Do NOT use random sleeps as the primary instrument.** Timing jitter in a fixture
manufactures flakes, and this project already fights them: a 1.15 % race in `reactor_os`
(#73), a teardown test failing 2-in-26 under load (#69), and 13 park points that hung with
zero output (`9254e85`).

★★ **The hazard is semantic, not temporal.** Model it as such:

- **per-client serialisation** — all verbs on one client take one lock, so a second verb
  on the same client cannot proceed;
- **an uninterruptible hold** — a verb the test wedges does **not** return until released,
  and is not cancellable;
- ⇒ the assertions become exact: *"a sibling isolate on the same client is blocked"*,
  *"an isolate on a different client made progress"*, *"the wedged verb consumed no
  cancellation"*.

The machinery already exists — `l1_mean`'s `VerbHold`, `StartGate`, `Latches` and
`MockRmBackend::gate`. This is teaching an existing hold to model RM's serialisation, not
new infrastructure.

Randomised delay is kept as a **secondary** fuzz behind `KAYFABE_SLOW=1`, never as the
thing an invariant rests on.

### 2.0 ★★★ BUILT, 2026-07-29 — and the measurement §2 asked for

Built as `kayfabe_mocks::ClientLock` (one lock per `IsolateId`, taken by
`MockRmBackend::gate` for the whole verb, released by a `ClientGuard` on every exit
including the `Err` paths), plus `RmRecorder::parked_verbs` (the ranked-lock mask a verb
parked with — R1 *across* the wait, which `Worker::execute`'s entry assert cannot see) and
`ClientLock::wait_until_blocked` (a progress **edge**, the `VerbHold::wait_until_pending`
idiom, so nothing sleeps). Asserted by `l1_mean`'s
`a_verb_wedged_on_one_rm_client_blocks_its_sibling_and_no_other_client`,
`a_wedged_verb_is_not_cancelled_and_still_holds_its_clients_lock` and
`every_poll_around_a_wedged_client_is_bounded_and_delivers_only_its_own`.

**★★ It is opt-in (`RmRecorder::serialize_per_client`, default off) and that is the
finding, not a compromise.** Forced on for one full workspace run: **12 tests stop
terminating, 0 assertions fail.** Every one is a *liveness* claim of the form *"N pool
workers of one isolate have N verbs in flight at once"* — `retry_ledger` arms one hold
**per pool slot** and requires them all pending; `l1_mean`'s composed run requires three
sibling threads of the witness `Proc` to finish while two of its verbs are parked;
`l1_verb_seam::progress_under_pending_verb_intra_proc` states it directly. The full table
is on the field's own docs.

That claim is **stronger than this design's**. §6.6 of `l1_os_shell.md` already states
I-NOAMP as a **cross-process** obligation and says so in as many words: *"on real
hardware, process A's slow RM call already makes process B's RM call wait. We do not
create this property and we cannot delete it. We can only amplify it."* The suite's
intra-proc progress claim was passing only because the double returned promptly by
construction.

**Open, and an owner call — not fixed here.** Either the intra-proc claim is renegotiated
in `l1_concurrency.md` §3.5/§7.2 (the pool buys latency isolation, not wire concurrency —
which `DEFAULT_POOL_WORKERS`' own docs already say) and those 12 tests are rewritten
around the weaker true property, or the flag stays off and the divergence stays recorded.
**No test was narrowed to get green**; the flag exists so the hazard is testable *today*
without silently editing twelve assertions.

#### ★★★ ANSWERED ON HARDWARE, 2026-07-29 (`93bc38d`) — and the answer is neither hypothesis

This section said only a real host could settle it. A real host has. RTX 3060 (GA104),
NVIDIA open 580.159.04, **800 `alloc_vaspace` + `free` pairs**, three configurations:

| configuration | wall | speedup |
|---|---|---|
| 1 worker, sequential | 1610 ms | 1.00× |
| 1 isolate × 4 workers (**one** RM client) | 1602 ms | **1.00×** |
| 4 isolates × 1 worker (**four** clients, four processes) | 1610 ms | **1.00×** |

Ideal is 4.00×. **Neither the worker pool NOR separate RM clients buys any alloc/free
throughput.** The binding constraint is the **device-global API write lock held across the
GSP RPC**, not the per-client lock. So the working hypothesis carried into the isolate task
— *"parallelism must come from multiple clients, not multiple workers on one client"* — is
**false for this verb class**, and so is the belief that the pool provides it.

⇒ **The 12 tests are NOT refuted, and none was touched.** The double is now wrong in the
*other* direction: `ClientLock` stops a sibling from **entering**, whereas real RM accepts it
and queues it in the kernel — so both verbs genuinely are in flight, and the tests' liveness
claims hold. What does not hold is the *throughput* they imply, and **no test asserts
throughput**. Scope honestly: this measures alloc/free only, the class that takes the lock in
WRITE; a read-mostly or doorbell-path verb may behave differently and has not been measured.

★ **Instrument warning, because it nearly produced the opposite headline.** The first version
of this measurement counted overlapping request *intervals* and reported ~460 concurrent in
the one-client case — reading as "the pool DOES buy concurrency". It does not; an interval
spans the socket round trip. **The sequential baseline is what made the numbers readable, and
it was missing from the first version.** Any future concurrency claim here needs that baseline
in the same table.

### 2.1 ★ The mock must lie where the real host lies

`MockRmBackend` validates handles against its own per-isolate namespace. `HostHandle`'s
docs state a real host does **not** — RM mints from one base, so the same raw value is
live and unrelated in a sibling isolate's client. Until the double reproduces that, every
handle-boundary test is optimistic. **This is a known, specific divergence: fix the double
rather than trusting it.** (`07da582` is the instance that exposed it.)

**★ FIXED, 2026-07-29.** `MockRmBackend::check` now resolves on the **host-visible** part
of the value (`kayfabe_mocks::HOST_RAW_MASK` — the mock's high lanes are instrumentation,
the low field is the per-client sequence every client mints from the same base, exactly as
RM does from `RS_CLIENT_HANDLE_BASE`). A sibling client's handle whose value is live here
is **served against the local twin** and recorded as a `BystanderHit`; the twin is
destroyed, which is the actual damage. A value live nowhere is still `BadHandle`.

Two things this changed, and neither was a weakening:

- **What still catches it** is `Worker::execute`'s foreign-handle gate (recorded
  provenance, refused before any verb) and `HostLedger::free_of_unknown` (the `Free` is
  logged with the handle as *presented*, so the reach is named and the destroyed twin
  stays outstanding forever). Both are audits of what happened rather than the backend's
  luck. `kayfabe_mocks`' own
  `a_sibling_clients_live_raw_value_is_served_exactly_as_a_real_host_would` pins all five
  steps.
- **Nothing in the suite broke** — 748/748 both ways. Measured, not assumed: no existing
  test was relying on the backend refusal in a case where a twin existed. The mean tests
  now additionally assert `bystander_hits == []` through the composed runs, which is
  non-vacuity for the gate: the door is open in the double and production never reaches
  it.

★ The remaining door is `Worker::with_rm`, the documented escape hatch, which skips the
foreign-handle gate by design. It is bring-up-only and the ledger still catches it, but it
is now the *only* place a cross-namespace reach can execute.

### 2.2 ★★★ ANSWERED ON HARDWARE, 2026-07-29 — and the answer is neither hypothesis

§2.0 left one question open and said only a real host could settle it: does the worker
pool buy **wire concurrency**, or only latency isolation? The working assumption when the
real isolate was built was the second, with the corollary that *parallelism must therefore
come from multiple clients* — i.e. from per-`(Proc, GpuId)` isolates.

**Both halves are false for the alloc/free verb class.** Measured with
`kayfabe-rm-ladder --concurrency` on RTX 3060 / NVIDIA open 580.159.04, 800
`alloc_vaspace` + `free` pairs, three configurations of the same total work:

| configuration | wall time | speedup |
|---|---|---|
| 1 worker, sequential | 1610 ms | 1.00x (baseline) |
| 1 isolate, 4 workers — **one** RM client | 1602 ms | **1.00x** |
| 4 isolates, 1 worker each — **four** RM clients, four processes | 1610 ms | **1.00x** |

Ideal would be 4.00x. Neither arrangement moves the number at all: ~2 ms per alloc+free
pair regardless of how many workers, clients or processes are issuing. The bottleneck is
**device-global**, not per-client — RM takes the global API lock in WRITE for every
alloc/free and holds it across the GSP RPC, which `DEFAULT_POOL_WORKERS`' own docs already
cite (`ogkm-610:`/`ogkm-580: .../rmapi/rmapi.c:53-58`, `:535`;
`ogkm-610: .../rmapi/alloc_free.c:1714-1718`, `ogkm-580: :1692-1696`). The per-client
write lock is real, and for this verb class it is *not the binding constraint*.

**What this does and does not say.** It is a measurement of alloc/free, which is precisely
the class that takes the API lock in WRITE. Verbs that take it in READ are not measured and
may well parallelise; nothing here licenses "RM never parallelises anything". A second
instrument caveat, recorded because it nearly produced the wrong headline: the first
version of this experiment counted *overlapping request intervals* and found ~460 of them
in the one-client case, which reads as "the pool DOES buy concurrency". It does not — an
interval spans the socket round trip as well as the ioctl, so intervals overlap while the
ioctls inside them serialise. **The sequential baseline is what made the numbers
readable**, and it was not in the first version.

#### ★ What it means for §2.0's twelve tests

They are **not refuted by hardware**, and the reason is that the double is wrong in the
*other* direction now. `ClientLock` models RM's serialisation as a lock that stops a
sibling **entering**; real RM accepts the sibling's ioctl and queues it in the kernel. From
our side both verbs are genuinely in flight, so the twelve liveness claims — *"sibling
threads of one `Proc` complete"* — hold against a real host. What does not hold is the
*throughput* those tests imply, and no test asserts throughput.

So: the double was too polite before `15651b1` and is too strict after it, and the true
constraint is at a different granularity than either. The flag stays opt-in, the twelve
tests stay unmodified, and the honest summary is that **the pool and the client split are
both blast-radius mechanisms, not performance ones** — which is what `DEFAULT_POOL_WORKERS`
says and what the isolate-per-`(Proc, GpuId)` split was designed for (#14).

---

## 3. ★★ DECISION — build the real isolate next, and force the question

No gate substitutes for building it. Shape is the C's Mode-1 architecture, which is
proven: **one host process per guest `mm`**, memfd migration, double-mmap, demand-fault
(`isolate_architecture`). Requirement carried from this session: **multiple ioctls must
execute in parallel without hanging** — which is what per-`(Proc, GpuId)` isolates plus an
N-worker pool exist for.

**Order:**

1. §2's deterministic blocking semantics — so R1/F1/I-NOAMP are tested against the real
   hazard **before** host code is written against assumptions.
2. The real isolate, validated against `/dev/nvidiactl` on the bench **as it is built**.
3. L2-Q under §1's design, by then with one fewer unknown.

## 4. ★★ DECISION — the run ladder exists from day one

`first contact` currently has no plan: the C has a bring-up ladder and
`C: scripts/run_mode2_vm.sh`; the Rust has neither. Given that **every** design here has
been wrong ~6 times per stage against a *cooperative* fixture, meeting a real driver
without a ladder means debugging several failures at once with no idea which layer owns
them.

The ladder is cheap and is written **before** the first bench run, not after — each rung
naming what is attempted and what "working" looks like, so a failure localises to a layer.

## 5. On size — a stated concern, and the honest reading

The tree is ~45 k implementation against ~53 k test. That ratio looks healthy, but with
the host side entirely mocked, **some of it is machinery validated by a fixture written to
satisfy it**. Building the real isolate is as much an opportunity to retire mock
complexity as to add code, and that should be looked for rather than assumed.

## 6. Not decided here

- GSP §11-O7a — **downgraded, not closed.** The C never sends `RUN_CPU_SEQUENCER` at all
  (grep: zero hits in `C: src/qemu/`), yet handles GSP reload via `gsp_reloaded`, WPR2
  re-raise and `GSP_INIT_DONE` re-post. So "reachable only via `_kgspRpcRunCpuSequencer`"
  is a true statement about one handoff and **not** a requirement for resume. Open
  question, not a blocker.
- #83 (mutation runs self-contaminate), #87 (a legitimate read-native exit scores as the
  N13 counter), #85, #73, #78, #68 — all still owner calls.

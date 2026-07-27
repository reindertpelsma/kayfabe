# What the QEMU ≥ 10.2 floor actually buys — a source-verified facility inventory

**What this file is.** The answer to the open task `../design/l1_os_shell.md` §10 L2-Q item 1
left behind: *"what else the floor buys us. Now that ≥ 10.2 is guaranteed, inventory the
facilities we were planning to hand-roll against 9.2 and take the upstream ones instead. Do this
**before** writing the adapter, since it changes what gets written."*

**Method, and its limits.** Every row below was read from the **`v10.2.0` tag** and, where the
question is *"is this a floor benefit or was it always there"*, diffed against **`v9.2.0`**.
Nothing here was run. This is a **source round, not a bench round** — it has no `[measured]`
tags of its own, and where it contradicts a measurement it does so by showing that the measured
path is not the path a QEMU adapter takes, never by re-measuring.

**Why it is a reference and not a design doc.** Same rule as `qemu_bql_spike.md`: where this file
and a design doc disagree about *what upstream does*, this file wins and the design doc gets
amended. It has no authority over what we *should* do — only over what is *there*.

Tags: **[src]** read from the named file at the named tag, **[inferred]** a conclusion drawn from
those, **[open]** not settled here and the experiment is named.

> **★ The discipline this file is an instance of.** §6.3.1 exists because a design doc named
> `memory_region_clear_global_locking()` and that function had been deleted five years earlier.
> Every row below therefore carries a `file:line`, and the rows that *cannot* — because they are
> about behaviour under load rather than about the presence of a symbol — say so and name the
> experiment instead of guessing. **Two live instances of the same decay were found while writing
> this file** and are recorded in §12, because they show the rot is ongoing rather than historical.

---

## 1. The table

| # | Facility | What our design planned | What upstream ≥ 10.2 actually provides (`file:line`, v10.2.0) | Verdict |
|---|---|---|---|---|
| 1 | **BQL-free MMIO dispatch** | apply `memory_region_clear_global_locking()` (dead since 5.2) → then: carry a ~4-line backport of 10.2's replacement onto 9.2 → then: require ≥ 10.2 (`c3ec258`) | `memory_region_enable_lockless_io(MemoryRegion*)` — `include/system/memory.h:2354`; field `bool lockless_io` `:836`; honoured at `system/physmem.c:3196-3209` (`if (!bql_locked() && !mr->lockless_io) bql_lock();`) | **take upstream's** — already the decision; §2 adds the constraint the decision did not state |
| 2 | **Reentrancy-guard pairing** | §9.3 gate: a region marked lockless *"without `disable_reentrancy_guard` on the same device"* is a CI failure — a **two-symbol discipline** | **one function does both**: `system/memory.c:2567-2580` sets `lockless_io` *and* `disable_reentrancy_guard` in the same body, with a comment saying why | **take upstream's** — the pairing is no longer ours to maintain; §2 re-specifies the gate, which currently describes code that would be wrong |
| 3 | **ioeventfd for the doorbell** | `memory_region_add_eventfd()`; handler *"runs on the main loop under the BQL, so it frees the vCPU and not the service"* — L2-Q task 4 carries a hand-off discipline to compensate | `include/system/memory.h:2356+` / `system/memory.c:2582-2616`. **KVM registration passes only the fd**: `accel/kvm/kvm-all.c:1889-1905` calls `kvm_set_ioeventfd_mmio(event_notifier_get_fd(e), …)` and installs **no read handler**. The read side belongs to whoever wants it | **take upstream's, and delete the compensating discipline** — §3. Not a floor benefit (present in 9.2); a *reading* benefit |
| 4 | **`raise_irq` as irqfd** | contractually irqfd-shaped, never `msix_notify()` | `kvm_irqchip_add_irqfd_notifier_gsi()` `include/system/kvm.h:498`; `msix_set_vector_notifiers()` `include/hw/pci/msix.h:46-49`; `event_notifier_set()` is one `write(2)` — `util/event_notifier-posix.c:107-119` | **take upstream's** — but **not a floor benefit**: `msix.h`'s only 9.2→10.2 delta is `nentries` widening to `uint32_t` |
| 5 | **The GPA window (coarse memslot)** | our own `MAP_NORESERVE` reservation; `MAP_FIXED` placement inside; never `munmap` a sub-range | `memory_region_init_ram_ptr()` `include/system/memory.h:1531` → `qemu_ram_alloc_from_ptr()` sets **`RAM_PREALLOC`** (`system/physmem.c:2566-2571`), and PREALLOC blocks are **skipped** by `qemu_ram_remap` (`:2684-2685`) and by `reclaim_ramblock` (`:2589-2591`) | **keep ours** for the mmap; **take upstream's** for the seam — §5. Upstream *guarantees* it will not touch our VMA, which the design assumed and never cited |
| 6 | **One memslot per window** | `kvm_max_slot_size` unset ⇒ one region = one slot | `accel/kvm/kvm-all.c:111` `static hwaddr kvm_max_slot_size = ~0;` (only `kvm_set_max_memslot_size()` `:1466` moves it — no x86/arm64 caller); slot budget `s->nr_slots_max = kvm_check_extension(KVM_CAP_NR_MEMSLOTS)` `:2669` | **keep ours** — the rule holds; upstream supplies no new lever, and none is needed |
| 7 | **RO-memslot as the region-lock fallback** | priced at **1.49 µs** — *"a flags-only flip"* (§6.7 correction; `guest_memory_lock.md` §7.3). **★ 2026-07-27: that price is retracted — see §11.1's amendment; the flip does not exist at either layer** | **that path is unreachable through QEMU.** `flatrange_equal` compares `readonly` (`system/memory.c:251-260`), so `memory_region_set_readonly()` (`:2392-2400`) produces `region_del` + `region_add`; and even the flags path re-issues with `memory_size = 0` first (`accel/kvm/kvm-all.c:373-383`) | **★ REVERSE — upstream makes this HARDER.** §11.1. Strengthens decision #49 (uffd-WP); ~~corrects a price, not a rule~~ **— and as of 2026-07-27 it does not even correct a price: KVM refuses the flip outright (`-EINVAL`)** |
| 8 | **userfaultfd via `/dev/userfaultfd`** (GL9) | our own open + probe, in `kayfabe-linux-raw`; refuse loudly on a `USER_MODE_ONLY` fd | QEMU has the identical logic: `util/userfaultfd.c:28-59` prefers `/dev/userfaultfd`, *"because it has better permission controls, meanwhile allows kernel faults without any privilege requirement (e.g. SYS_CAP_PTRACE)"* — then **silently falls back to the syscall** at `:56` | **keep ours** — §6. Upstream **corroborates** the finding and **demonstrates the hazard**: its fallback is exactly the silent-`USER_MODE_ONLY` case GL9 refuses |
| 9 | **The reactor (epoll, our own thread)** | our own epoll loop; `ExecutorWaker` abstracted, *"QEMU impl: schedule a bottom half"* (§3.7) | `IOThread` (`include/system/iothread.h`), `aio_bh_schedule_oneshot` (`include/block/aio.h:402`), `qemu_set_fd_handler` (`include/qemu/main-loop.h:227`) — all long predate 9.2 | **keep ours** — §7. But §3.7's *"in QEMU the executor is not our thread"* is **no longer forced** once dispatch is lockless; a BH is required only for BQL-requiring work |
| 10 | **io_uring** | B7: deliberately not `io_uring` (lock-free ring + kernel attack surface) | new in 10.2: `aio_add_sqe()` `include/block/aio.h:870`, `CqeHandler` `:66-72`, `aio_has_io_uring()` | **keep ours — B7 unchanged.** Upstream's is inside `AioContext`, which we do not use; its existence changes none of B7's three reasons |
| 11 | **Device reset (T4)** | `Spine::device_reset -> Vec<Proc>`; §6.6: guest-requested work runs on the calling thread with our locks dropped | `ResettableClass` three-phase (`include/hw/resettable.h:63-89`), `device_class_set_legacy_reset()` `include/hw/qdev-core.h:1002`. **`include/hw/resettable.h:50`: *"This whole API must only be used when holding the iothread mutex."*** | **take upstream's shape, with a named constraint** — §8. Reset arrives **BQL-held**; T4 must not block in the callback |
| 12 | **Migration / lifecycle** | §7.6 enumerates **eight** teardown triggers. Migration is not among them and is not discussed | `migrate_add_blocker(Error**, Error**)` `include/migration/blocker.h:32` — *"prevent all modes"*; declarative alternative `VMStateDescription.unmigratable` `include/migration/vmstate.h:191` | **take upstream's** — §9. One call at realize closes a gap the design has not opened |
| 13 | **CPR (`cpr-transfer`)** | nothing — the concept does not appear in the design | ≥ 10.x wires checkpoint-restart into RAM allocation itself: `cpr_name(mr)` / `cpr_delete_fd()` at `system/physmem.c:2504-2536`; `QEMU_PCI_SKIP_RESET_ON_CPR` `include/hw/pci/pci.h:234-235`; `include/migration/cpr.h` | **★ REVERSE — a NEW lifecycle event.** §11.2. Closed for free by row 12, but only if row 12 is taken deliberately |
| 14 | **Shareable guest RAM** (§4.4.1) | *"on QEMU it is `memory-backend-file`/`memory-backend-memfd` with `share=on`"* | unchanged for guest RAM. New in 10.x and **orthogonal**: `-machine aux-ram-share=on` → `current_machine->aux_ram_share` makes *auxiliary* RAM memfd-backed (`system/physmem.c:2499-2501`), and is skipped entirely when `host != NULL` (`:2498`) — i.e. it never applies to our window | **keep ours** — the precondition and the loud realize-time refusal stand, unchanged |
| 15 | **Guest-RAM discard hazard** | not discussed | `ram_block_discard_disable(bool)` `include/system/memory.h:3290` — the vfio-style opt-out; `ram_block_discard_range()` `system/physmem.c:4094` is the `madvise`/`fallocate` path it disables | **open question** — §10. Our window is `RAM_PREALLOC` (row 5) so the *remap* path skips it; the *discard* path's reachability for a guest-chosen GPA is `[open]` |
| 16 | **Cacheability / memory types** | `../reference/memory_cacheability.md` §5: the trait says nothing; KVM's `kvm_is_mmio_pfn()` silently drives it | no new facility. The only lever remains `memory_region_init_ram_device_ptr()` `include/system/memory.h:1559` (sets `ram_device`, "should not be included in a memory dump… operations incompatible with manipulating MMIO should be avoided") | **keep ours** — the floor buys nothing here |
| 17 | **In-tree Rust device bindings** | a hand-written adapter (`kayfabe-vmm-qemu`) over a thin C shell | **real and new**: a full `rust/` workspace at v10.2.0 — `rust/bql`, `rust/qom`, `rust/system`, `rust/hw/core`, `rust/migration`, `rust/util`, `rust/common`, `rust/chardev`, `rust/trace`. C-side support: `rust_bql_mock_lock()` / `bql_block_unlock()` `include/qemu/main-loop.h:253,304` | **keep ours** — §4. Three independent disqualifiers, the first fatal |
| 18 | **`gpa_read`/`gpa_write` as in-lock-legal** | §6.1: *"memcpy into an already-installed mapping"* ⇒ **yes**, in-lock legal | RAM path is a `memcpy` under RCU only (`system/physmem.c:3448` `RCU_READ_LOCK_GUARD()`, RAM branch at `:3370-3376`) — **but** the *same* entry point takes the BQL if the address lands on MMIO (`:3250`, `:3347` → `prepare_mmio_access`) | **★ keep ours, with a binding new constraint** — §11.3. The obvious implementation of a "yes" row can take the BQL for a **guest-chosen** address |

---

## 2. Rows 1–2 — the pairing is upstream's now, and §9.3's gate describes the wrong code

**[src]** `system/memory.c:2567-2580`, in full, because the whole finding is that it is one
function:

```c
void memory_region_enable_lockless_io(MemoryRegion *mr)
{
    mr->lockless_io = true;
    /*
     * reentrancy_guard has per device scope, that when enabled
     * will effectively prevent concurrent access to device's IO
     * MemoryRegion(s) by not calling accessor callback.
     *
     * Turn it off for lock-less IO enabled devices, to allow
     * concurrent IO.
     * TODO: remove this when reentrancy_guard becomes per transaction.
     */
    mr->disable_reentrancy_guard = true;
}
```

The owner's verification (`c0c6806`) noted that the two fields live on the same struct and
concluded that §9.3's *"one helper does both"* pairing gate is *"the natural shape rather than a
bolted-on discipline"*. **It is stronger than that: upstream's helper already is that helper.**
There is no pairing for us to maintain, and consequently:

> ### ★★ The gate must be RE-SPECIFIED, because as written it describes code that would be wrong
>
> §9.3's row says the gate fails when a region is marked lockless *"**without**
> `disable_reentrancy_guard` on the same device — or vice versa"*. Under ≥ 10.2 an adapter that
> touches `disable_reentrancy_guard` **at all** is doing something upstream already did, and a
> second write to it is at best redundant and at worst a divergence. The gate should be:
>
> - **exactly one call site** for `memory_region_enable_lockless_io`, in a single adapter helper;
> - **zero** occurrences of `disable_reentrancy_guard`, `bql_unlock`, and any hand-rolled
>   unlock/lock around a dispatch, anywhere in the tree.
>
> That is *simpler* than the two-symbol version and strictly harder to get wrong. It also keeps
> the 47 %-silent-drop finding load-bearing: what it now forbids is the **hand-rolled**
> alternative, which is the only way to reproduce that failure on ≥ 10.2.

**Three precision corrections to how §6.3.1 and §9.3 describe the guard**, all `[src]`, none of
which change a conclusion:

1. **The guard's *state* is per-device; the *opt-out* is per-region.** `engaged_in_io` lives on
   `mr->dev->mem_reentrancy_guard` (`system/memory.c:547`), `disable_reentrancy_guard` on the
   `MemoryRegion` (`include/system/memory.h:869`). §6.3.1's *"keyed on the device, not the
   region"* is right about the state and §9.3's *"on the same device"* is wrong about the flag.
2. **⇒ The pairing must be applied to EVERY trapped region of the device, not just the hot one.**
   A region *without* the flag still sets `engaged_in_io` and still returns
   `MEMTX_ACCESS_ERROR` to a concurrent access; a region *with* it never sets the flag, so it can
   neither block nor be blocked. A device with a lockless BAR0 and a stock BAR2 therefore retains
   **both** hazards on BAR2 — the BQL *and* the silent drop — while passing any gate phrased
   per-device. This is the sharpest practical consequence in this file.
3. **The guard never applied to RAM-like regions in the first place.** `system/memory.c:545-546`
   skips it when `mr->ram_device || mr->ram || mr->rom_device || mr->readonly`. So a BAR1 aperture
   backed by a RAM region was never guarded and never serialised — which is a *narrowing* of
   §6.3.1's *"any two vCPUs touching the same device collide"*, and it means the R1/R3/R5
   promotion to correctness-requirements was **already** in force for RAM-backed BARs before
   lockless IO was taken.

**And one thing lockless IO does *not* do**, stated because the code makes it easy to miss.
**[src]** `system/physmem.c:3200`: `if (!bql_locked() && !mr->lockless_io) bql_lock();`. The flag
suppresses *acquiring* the BQL; it does not *release* one the caller already holds. On the
vCPU/KVM path that is the whole story — `accel/kvm/kvm-all.c:3238,3247` are commented *"Called
outside BQL"* for `KVM_EXIT_IO` and `KVM_EXIT_MMIO`, exactly as the 9.2 spike found. **But any
dispatch we originate from a BQL-holding context of our own (a bottom half, a reset callback, a
monitor command) still runs the handler with the BQL held**, and the flag will not tell us. That
is a reason to keep §6.3 enforcement item 3 — the written list of what the trap path may call —
rather than to retire it as the floor decision made tempting.

---

## 3. Row 3 — **the ioeventfd's read side is OURS**, and that deletes a discipline

**[src]** `accel/kvm/kvm-all.c:1889-1905`:

```c
static void kvm_mem_ioeventfd_add(MemoryListener *listener,
                                  MemoryRegionSection *section,
                                  bool match_data, uint64_t data,
                                  EventNotifier *e)
{
    int fd = event_notifier_get_fd(e);
    ...
    r = kvm_set_ioeventfd_mmio(fd, section->offset_within_address_space, ...);
```

That is the entirety of QEMU's involvement. **The `EventNotifier` is ours** — we construct it, we
pass it to `memory_region_add_eventfd`, and QEMU hands its descriptor to KVM. **QEMU installs no
read handler and never reads it.** Nothing puts that fd on the main loop unless the device asks,
by calling `qemu_set_fd_handler`/`aio_set_fd_handler` itself.

> ### ★★ Consequence — `qemu_bql_spike.md` §7's normative box is over-general, and L2-Q task 4 shrinks
>
> The spike measured *"the ioeventfd handler runs on the main loop with the BQL held
> (`bql_locked()` = 1)"* and generalised it to *"ioeventfd frees the vCPU, not the SERVICE"*, from
> which §6.3.1 and §6.6 item 2 derive a hand-off discipline and L2-Q task 4 carries an obligation.
> **The measurement is a property of how the spike wired its read side, not of ioeventfd.**
>
> **The shape that makes the caveat vacuous:** register the eventfd's descriptor **in our own
> reactor's epoll set** (§3), as one more `CompletionSource`. It is already a counter-shaped,
> level-triggered, drainable primitive — precisely the §3.4 source contract — and it arrives on
> the reactor thread, which holds no lock, has no BQL and touches no core state. The doorbell then
> frees the vCPU **and** the service, with no discipline to maintain and no main loop involved.
>
> ★ **This does not weaken the I-NOAMP argument, it discharges it structurally**, which is the
> move §6.6 says it prefers. And it costs nothing: the reactor exists, the source kind exists, and
> the alternative (a QEMU BH that must hand off to our executor) is strictly more machinery.

**What survives unchanged from the spike**, because none of it was about the read side: the
no-datamatch registration argument; the coalescing-is-safe-*here* reasoning; the *"quote the
availability result, not the speedup"* rule; and the open decision (L2-Q task 4) about whether to
import the C's O(live channels) `GP_PUT` scan, which §6.3.1 rightly refuses to inherit.

**[src] One BQL fact about ioeventfd that does not change:** *registration* takes the BQL.
`memory_region_add_eventfd` (`system/memory.c:2582-2616`) runs a memory transaction, and
`memory_region_transaction_commit` asserts `bql_locked()` (`:1148`). Same for every topology
mutation — subregion add/remove, `memory_region_set_enabled`, `memory_region_set_readonly`. That
is the coarse tier of §6.7's two-tier table and it confirms both the **NO** classification and
§6.3's *"exactly one BQL acquisition site in the adapter"*.

---

## 4. Row 17 — the in-tree Rust workspace is real, and we still cannot use it

This is the most attractive-looking row in the file and the one most worth being careful about.
**[src]** v10.2.0 ships `rust/` with `Cargo.toml`, `bindings`, `bits`, `bql`, `chardev`, `common`,
`hw` (`char/pl011`, `timer/hpet`, `core`), `migration`, `qom`, `system`, `trace`, `util`, and C-side
support hooks (`rust_bql_mock_lock`, `bql_update_status`, `mutex_is_bql`, `bql_block_unlock` —
`include/qemu/main-loop.h:253-304`).

**Three disqualifiers, in descending order.**

1. **★★ It is in-tree only, which means it is a fork — the exact thing `c3ec258` just deleted.**
   These are workspace-internal crates built by meson as part of QEMU; there is no published,
   versioned binding crate an out-of-tree device can depend on. Using them means our device's
   source lives inside a QEMU checkout. The floor decision's own accounting — *"a tracked patch, a
   rebase every release, a build script that must reproduce it, and a supply-chain claim we make
   to every user forever"* — applies verbatim, and more heavily, to carrying a device rather than a
   4-line patch. **Requiring a version was chosen precisely to avoid this shape.**
2. **The abstraction is the wrong one for us.** `rust/bql/src/cell.rs` provides `BqlCell` /
   `BqlRefCell`: interior mutability whose soundness argument *is* the BQL, backed by
   `bql_block_unlock()`, whose doc reads *"The Big QEMU Lock (BQL) is used to provide interior
   mutability to Rust code, but this only works if other threads cannot run while the Rust code has
   an active borrow"* (`include/qemu/main-loop.h:292-303`). Our device is the opposite by
   construction: `Device` takes `&self` (decision #42), does its own ranked locking (R1/R3/R5), and
   **runs with no BQL at all** on every lockless region. Adopting the idiom would import an
   assumption we have spent a milestone designing out.
3. **There is no PCI support.** `rust/hw` contains `char`, `core` and `timer` only — no `pci`. A
   PCI device is not expressible there at v10.2.0.

**What is worth taking from it anyway, at zero cost:** `rust/system/src/memory.rs` is 195 lines and
its `MemoryRegionOpsBuilder` (`:37-120`) is a clean, const-buildable encoding of `MemoryRegionOps`
including the `valid`/`impl` access-size and unaligned knobs. **[inferred]** It is a good model for
our adapter's own ops builder — a design to read, not a dependency to add. Note also
`MemoryRegion::init_io` is the *only* wrapped constructor: everything else (ram_ptr, eventfd,
lockless_io) is reachable only through the raw bindgen `bindings` module, so even in-tree the
safe-wrapper surface would not have covered what we need.

---

## 5. Row 5 — upstream *guarantees* it will not touch our window, and the design never cited it

The C's shape, which §6.7 endorses and §4.4 formalises, is: **we** mmap a large
`MAP_ANONYMOUS|MAP_NORESERVE` reservation, hand the pointer to the VMM once, and every publication
is a `MAP_FIXED` placement inside it. On QEMU the seam is `memory_region_init_ram_ptr()`
(`include/system/memory.h:1531`) → `qemu_ram_alloc_from_ptr()`, which **[src]** sets `RAM_PREALLOC`
(`system/physmem.c:2566-2571`).

`RAM_PREALLOC` is what makes the shape safe, and it does so by three explicit early-outs:

| Path | `file:line` | Behaviour on a PREALLOC block |
|---|---|---|
| `qemu_ram_remap` (memory-error recovery) | `system/physmem.c:2684-2685` | `;` — no-op |
| `reclaim_ramblock` (block destruction) | `system/physmem.c:2589-2591` | `;` — **no `munmap`**; the VMA stays ours |
| shared/aux-ram promotion | `system/physmem.c:2498` | the whole `!host` branch is skipped |

**[inferred]** So QEMU will never remap, discard-and-refault, munmap, or re-back our window. The
design assumed this; it is now cited. Note the direction of the guarantee: it also means
**teardown is entirely our problem** — dropping the `MemoryRegion` frees nothing, so §7.6's
lifecycle owns the reservation from creation to unmap, with no VMM-side backstop.

**[src]** `assert(!host ^ (ram_flags & RAM_PREALLOC))` (`:2494`) — the two are the same thing;
there is no way to hand QEMU a pointer *and* have it manage the memory.

---

## 6. Row 8 — upstream corroborates GL9, and demonstrates the hazard it refuses

**[src]** `util/userfaultfd.c:28-59`. QEMU opens `/dev/userfaultfd` first and falls back to the
syscall, with this comment:

> *"Make /dev/userfaultfd the default approach because it has better permission controls, meanwhile
> allows kernel faults without any privilege requirement (e.g. SYS_CAP_PTRACE)."*

That is `guest_memory_lock.md` §1.2's load-bearing deployment finding, independently reached by
upstream, including the `CAP_SYS_PTRACE` half. **This is the second time in two rounds that
upstream has stated one of our conclusions in its own words** (the first being
`memory_region_enable_lockless_io`'s doc comment), which is a mild but real signal that the
analysis is on the right track.

**Verdict is still keep ours**, for two reasons that pull in the same direction:

1. **It is not a hypervisor capability.** §6.8's rule — *"a `Vmm` method must name something only
   the VMM can do"* — applies to the implementation as much as to the trait. `UFFDIO_REGISTER` on
   our own window VMA needs no VMM cooperation, so it belongs in `kayfabe-linux-raw`, and QEMU's
   `uffd_*` helpers are internal C symbols we would be reaching across an adapter to call.
2. **★ Upstream's fallback is precisely the failure GL9 exists to refuse.** `util/userfaultfd.c:56`
   silently drops to `syscall(__NR_userfaultfd, flags)` when `/dev/userfaultfd` is absent. For
   QEMU's postcopy use that is correct — it wants user-mode faults. For **ours** it is fatal and
   silent: an unprivileged syscall-created uffd is `USER_MODE_ONLY`, which **[measured, GL9]** does
   not trap guest writes, so the lock appears held and protects nothing. **A fallback is the right
   default for upstream's use and the wrong one for ours** — which is a better argument for owning
   the open than any style preference.

**★ A hazard the design does not name: uffd registrations do not compose.** **[inferred]** A VMA
range can be registered to one uffd. QEMU calls `uffd_register_memory` for postcopy live migration
(`util/userfaultfd.c:158`) and for write-tracking migration (`RAM_UF_WRITEPROTECT`,
`include/system/memory.h:244`). If either ever ran over the RAMBlock containing our window, one of
the two registrations loses. Row 12's `migrate_add_blocker()` closes this as a side effect — which
is a second, independent reason to take it.

---

## 7. Row 9 — the executor need not be QEMU's thread

§3.7 says: *"in QEMU the executor is **not** our thread: it is the main loop / bottom-half
context… QEMU impl: schedule a bottom half."*

**[inferred]** That was a consequence of BQL-held dispatch, and it is no longer forced. With
lockless IO, a trapped access arrives on the vCPU thread holding **no** QEMU lock; with row 3, a
doorbell arrives on our reactor thread holding no QEMU lock. Neither needs a BH to reach the
executor. **The executor can be our own thread on QEMU, exactly as in the harness, and
`ExecutorWaker::wake` can be the same condvar/eventfd write on both backends.**

A bottom half remains necessary for exactly one thing: work that must run **with the BQL held**,
i.e. the coarse memory-plane tier (§2's last paragraph) and anything else that calls
`memory_region_transaction_commit`. That is already §6.3's single acquisition site, and a BH is one
legal way to spell it.

**Keep the `ExecutorWaker` trait regardless** — it is four lines, and §3.7's argument that it makes
L2 an adapter rather than a re-plumb is unaffected by which implementation each backend picks.
**Owed edit: §3.7's example should stop asserting the BH as *the* QEMU impl.**

---

## 8. Row 11 — reset arrives BQL-held

**[src]** `include/hw/resettable.h:50`: *"This whole API must only be used when holding the iothread
mutex."* (`iothread mutex` is the BQL's former name; the file was not updated in the rename.)

Consequence for §7.6 **T4** (`Spine::device_reset`, decision #28): the reset callback is **not** a
lock-free context in the §6.6 sense. It is the one entry path where a QEMU-global lock is held and
where lockless IO does not help, because the flag suppresses *taking* the BQL, not *having* it.
Therefore:

- T4 must be **latch-and-defer**: record the reset, return, and let the executor do the reclamation
  work — which is exactly what §6.6's *"background work with no caller to bill it to runs on the
  executor"* already prescribes, so this is a constraint the design satisfies by accident rather
  than a change. It is written down here so it is satisfied on purpose.
- The eight-trigger property (§7.7) is unaffected; only the *thread* T4 runs on is pinned.

**[src]** `device_class_set_legacy_reset()` still exists at `include/hw/qdev-core.h:1002` and the
`legacy_reset` field at `:167-172` carries a *"deprecated… TODO: remove once every reset callback is
unused"* comment. **Use `ResettableClass` three-phase, not the legacy hook** — the deprecated one is
a floor-ratchet liability in exactly the sense of the floor decision's residual (2).

---

## 9. Row 12 — one call closes a gap that is not in the design

§7.6 enumerates eight teardown triggers. **Migration is not one of them, and neither is CPR.**
Both are states in which QEMU expects a device to serialise itself and be reconstructed elsewhere,
and neither is meaningful for us: our state includes live host RM clients, isolate processes, GPU
VAs and a `MAP_FIXED` window into another process's `mm`.

**[src]** `include/migration/blocker.h:32` — `int migrate_add_blocker(Error **reasonp, Error **errp)`,
documented as *"prevent **all** modes of migration from proceeding"*. Called once at realize, with a
reason string, it closes rows 12 and 13 and the §6 uffd-composition hazard together.

**[src]** The declarative alternative is `VMStateDescription.unmigratable`
(`include/migration/vmstate.h:191`). **[inferred]** The blocker is preferable: it produces an error
naming *our* reason at the moment migration is attempted, and it is greppable as a deliberate act.

> **★ This is a "take upstream's" that deletes design we have not written yet** — which is the best
> kind, and the reason the L2-Q task asked for this inventory *before* the adapter. The alternative
> is discovering at L3 that a `migrate` command produced a half-serialised device.

---

## 10. Row 15 — the discard question, left open honestly

**[src]** `ram_block_discard_disable(bool)` (`include/system/memory.h:3290`) is the vfio-style
global opt-out that prevents uncoordinated discard (balloon) and, in its coordinated variant,
virtio-mem. **[src]** The discard itself is `ram_block_discard_range()`
(`system/physmem.c:4094-4119`), which `madvise(DONTNEED)`/`fallocate(PUNCH_HOLE)`s a range of a
`RAMBlock`.

**The question:** can a guest cause a discard on a GPA inside **our** window — e.g. by handing a
balloon device a page that resolves into our RAMBlock? If so, the backing behind a `MAP_FIXED`
placement is zeroed under us: silent data loss of exactly the shape §12.13 catalogues.

**Why it is `[open]` and not answered here:** row 5 shows `RAM_PREALLOC` short-circuits
`qemu_ram_remap` and `reclaim_ramblock`, but `ram_block_discard_range` has **no `RAM_PREALLOC`
check** — it gates on alignment, `rb->fd`, and hugepage/shmem properties. Whether the balloon path
can ever resolve a GPA to a device-owned RAMBlock is a question about `virtio-balloon` and
`qemu_ram_block_from_host`, which this round did not read.

> **The experiment that settles it** — cheap, and it belongs in L2-Q: bring up our device with a
> `virtio-balloon` present, have the guest balloon a page whose GPA lies inside our window, and
> assert either that QEMU refuses or that the page survives. If it does not survive, the answer is
> one line — `ram_block_discard_disable(true)` at realize, exactly as vfio does — and the reason to
> find out now is that the *symptom* is a zeroed backing at an arbitrary later time.

---

## 11. ★★ What ≥ 10.2 makes HARDER, or what a QEMU adapter invalidates

Three items. The first two are the reverse-direction findings the inventory was asked for; the
third is not about 10.2 at all but was found while verifying a 10.2 claim, and is the sharpest
thing in this file.

### 11.1 The RO-memslot fallback is **not** a 1.49 µs flip on QEMU

> **★★ AMENDED (2026-07-27) — this section conceded too much.** The sentence below grants that
> §6.7's correction *"is correct **about KVM**"* and faults only its reachability through QEMU.
> **[measured, KVM-direct, 2026-07-27]** it is **not correct about KVM either**: a flags-only
> `KVM_MEM_READONLY` change on a live slot returns **`-EINVAL`** (**[src]**
> `linux v7.1.0-rc6 virt/kvm/kvm_main.c:2075-2082`), the 1.49 µs figure was read off the
> **`noop`** row of the harness, and the harness's `flags` arm toggles dirty logging rather than
> read-only. See `region_lock_mechanism_study.md` §2.1–§2.2 and the retraction banner now at
> `../design/l1_os_shell.md` §6.7.
>
> **This makes §11.1's own conclusion stronger, not weaker.** Both layers refuse the flip, so
> the *"a KVM-direct measurement is not a QEMU measurement"* lesson below understates the case:
> here the KVM-direct measurement was not even a **KVM** measurement. The two reasons given
> below remain independently valid as source reads about QEMU and are left standing.

~~§6.7's *"★★ [measured] CORRECTION — a FLAGS-ONLY flip is 1.49 µs, not 230–460 µs"* is correct
**about KVM**, and it was measured on the KVM-direct harness.~~ **It is not reachable through QEMU's
memory API**, for two independent reasons, either sufficient:

1. **[src]** `flatrange_equal` compares `readonly` (`system/memory.c:251-260`). So changing it via
   `memory_region_set_readonly()` (`:2392-2400`) yields a FlatView whose range differs, and the
   listener emits `region_del` then `region_add` → `kvm_set_phys_mem(remove)` then `(add)`. That is
   a **DELETE (two `kvm_swap_active_memslots` + a shadow zap) plus an ADD**, i.e. the 230–460 µs
   p50 / 1–4 ms tail path, charged per vCPU.
2. **[src]** Even the flags-only slot path is not free: `kvm_set_user_memory_region`
   (`accel/kvm/kvm-all.c:373-383`) detects a changed `KVM_MEM_READONLY` bit and **issues the ioctl
   twice**, the first with `memory_size = 0` — a deliberate DELETE, per the cited KVM commit
   `75d61fbc`. And that path is only reachable from `log_start`/`log_stop` (`:733`, `:750`), which
   are dirty-logging transitions, not ours to trigger.

**What changes and what does not:**

- **`guest_memory_lock.md` §7.3's fallback is much more expensive than recorded** on the shipping
  backend. The measured 70.2 µs RO-memslot trap round-trip is a harness number; through QEMU it
  carries a device-wide DELETE+ADD.
- **Decision #49 (uffd-WP) gets *stronger*, not weaker.** It was decided on I-NOAMP grounds and
  happened to be 2.8× faster; the real QEMU-side ratio is larger than that.
- **Decision #37 (§6.7) is untouched.** The rule is about install/remove *frequency*; this is a
  correction to a price, and it moves in the direction the rule already points.
- **★ The methodological point, which is the durable one:** §6.7's correction was flagged as
  important because *"the error ran in the direction that wrongly discourages a legitimate design"*.
  This correction runs the **other** way — it made a fallback look affordable that is not — and the
  cause is the same in both cases: **a KVM-direct measurement is not a QEMU measurement.** M2-c
  building against the harness (decision #48) is right for the reasons given, *and* every constant
  it produces must be re-derived before it is quoted at a QEMU adapter.

### 11.2 CPR is a lifecycle event the eight triggers do not cover

**[src]** In ≥ 10.x, checkpoint-restart is not a bolt-on: `qemu_ram_alloc_internal` itself calls
`cpr_name(mr)`, `qemu_ram_get_shared_fd(name, &reused, …)` and `cpr_delete_fd(name, 0)`
(`system/physmem.c:2504-2536`), and PCI carries `QEMU_PCI_SKIP_RESET_ON_CPR`
(`include/hw/pci/pci.h:234-235`). `cpr-transfer` execs a **new QEMU binary** over the old one,
preserving RAM and passed descriptors.

For us that would mean: a new process, our isolates' parent gone, our reservation's `mm` gone, our
uffd gone, our epoll set gone. **It is not a teardown trigger we can implement; it is one we must
refuse.** Row 12's `migrate_add_blocker()` refuses all modes including CPR, which is why that call
is a requirement rather than tidiness.

**[inferred]** Our window is `RAM_PREALLOC` (`host != NULL`), so the `!host` CPR/shared-fd branch at
`:2498` never runs for it — meaning CPR would silently *not* preserve our window rather than fail
loudly. That is the worse failure mode, and it is the argument for blocking rather than trusting.

### 11.3 ★★ `gpa_read` / `gpa_write` can take the BQL — for a guest-chosen address

§6.1 classifies `gpa_read`/`gpa_write` **in-lock legal**, on the grounds that they are *"a memcpy
into an already-installed mapping"*, and §6.3 makes that legality conditional: in-lock-legal methods
*"MUST be implementable as a primitive that cannot reach VMM-global API"*. **The obvious QEMU
implementation violates that condition, and the violation is guest-steerable.**

**[src]** `address_space_write` takes `RCU_READ_LOCK_GUARD()` and no BQL (`system/physmem.c:3448`).
The **RAM** branch of the continue-step is a plain `memcpy` (`:3370-3376`). But the *same* entry
point, for a target that is **not** direct-access, calls `prepare_mmio_access(mr)` (`:3250` for
write, `:3347` for read) — and `prepare_mmio_access` takes the BQL unless the region opted out
(`:3196-3209`). Whether a given `gpa_write` is a memcpy or a BQL acquisition is decided by **which
memory region the guest physical address lands on**.

> ### ★★★ NORMATIVE (proposed) — the in-lock-legal RAM accessors must not go through `address_space_rw`
>
> The adapter's `gpa_read`/`gpa_write` MUST resolve the GPA to a **host RAM pointer** and memcpy —
> `address_space_map`/`memory_region_get_ram_ptr` at install time, cached against the window we
> already own — and MUST refuse a GPA that does not resolve to RAM. It MUST NOT call
> `address_space_rw`/`address_space_read`/`address_space_write` from an in-lock context.
>
> Otherwise a guest that steers one of our GPA accesses at an MMIO page turns a **rank-1-held**
> memcpy into a **BQL acquisition beneath our lock** — the exact ABBA of §6.3, constructed on
> demand, and invisible to every one of §6.3's four enforcement layers: it is not a "NO" row (1),
> not an acquisition site we wrote (2), and it *is* on the written list of functions the trap path
> calls (3) unless someone knew to look inside it.

> ### ★★ THREE CORRECTIONS to the box above, from the build that adopted it (2026-07-27)
>
> Recorded here because this file's rule is *"where this file and a design doc disagree about
> what upstream does, this file wins"* — so when the build read more of upstream, this file is
> where the reading lands. Full write-up: `../design/l1_concurrency.md` §12.43 §5.
>
> 1. **The rule is "prove RAM", not "refuse MMIO".** **[src]** `system/physmem.c:3010-3017` —
>    `io_mem_init` calls `memory_region_enable_lockless_io(&io_mem_unassigned)`, commented
>    *"Trivially thread-safe since memory accesses are rejected"*. So at v10.2.0 an **unassigned**
>    GPA does **not** take the BQL, while every ordinary device region does. A deny-list reasoned
>    from "MMIO is dangerous" is therefore both over-inclusive (unassigned is harmless) and
>    under-inclusive (it must enumerate every device). Only a positive allow-list is stable.
> 2. **★ Upstream has a facility that LOOKS like the fix and is not sufficient.**
>    **[src]** `MemTxAttrs.memory` (`include/exec/memattrs.h:46`, *"bus transactions are restricted
>    to normal memories… Access to devices will be logged and rejected"*) is honoured by
>    `flatview_access_allowed` (`system/physmem.c:3222-3238`), and it runs **before**
>    `prepare_mmio_access` in both continue-steps (`:3243` precedes `:3250`; `:3339` precedes
>    `:3347`). Setting it would refuse a device access with `MEMTX_ACCESS_ERROR` and take no lock.
>    **But its RAM test is `memory_region_is_ram(mr)`**, while `memory_access_is_direct` goes
>    through `memory_region_supports_direct_access`, which **excludes `ram_device`**
>    (`include/system/memory.h:3136-3151`, *"RAM DEVICE regions can be accessed directly using
>    memcpy, but it might be MMIO… So we treat this as IO"*). A `ram_device` region — which is
>    exactly what `memory_region_init_ram_device_ptr` produces for a VFIO-mapped BAR, row 16's
>    only lever — **passes `attrs.memory` and then takes the BQL**. ROMD regions have the same
>    shape on the write side. There is also no `MEMTXATTRS_MEMORY` constant at v10.2.0; the bit
>    would have to be set by hand. **Take the cached-pointer fix; the attrs flag is a trap.**
> 3. **The straddling case is the one a naive fix misses.** **[src]** `flatview_write_continue`
>    (`:3289-3315`) walks region by region, so a range that starts in RAM and runs into a device
>    window is a legal memcpy on step 1 and a `prepare_mmio_access` on step 2. A check on the
>    start address alone is not a fix.

**[inferred]** This does not say a bug exists today — the core touches `Vmm` in three places and
`gpa_read` once. It says the constraint must be written into the trait's rustdoc **before** the
adapter is written, which is the whole premise of doing this inventory early. It is also a second
worked instance of §6.1's own warning that the "yes" rows are legal *"only because their
implementations cannot reach VMM-global API — that is a binding requirement on the adapter, not a
property of the method name"*. The requirement now has a named way to be broken.

---

## 12. Two more instances of API decay, found in passing

§6.3.1's lesson — *"a named API in a design doc is a claim about a version, and it decays"* — was
drawn from a five-year-old deletion. Two live instances turned up while writing this file, offered
as evidence that the rot is current rather than historical:

1. **The header moved.** `include/exec/memory.h` (9.2) → `include/system/memory.h` (10.x). Every
   `[src]` citation in `qemu_bql_spike.md` and in §6.3.1 that names a 9.2 path is already stale as a
   *path*, though not as a *fact*. Same for `include/sysemu/kvm.h` → `include/system/kvm.h`.
2. **A signature changed.** `migrate_add_blocker_modes(Error **reasonp, Error **errp, MigMode mode,
   ...)` (variadic, 9.2) → `migrate_add_blocker_modes(Error **reasonp, unsigned modes, Error **errp)`
   (bit set, 10.2). A design doc that had named the 9.2 form would compile-fail rather than
   mis-behave — which is the *lucky* case, and not one to rely on.

**[inferred]** The practical rule this suggests: adapter-side `[src]` citations should carry the tag
they were read at (`v10.2.0 include/system/memory.h:2354`), not just a path — the floor decision
makes that a stable thing to write for the first time.

---

## 13. What this round did **NOT** establish

Stated so nothing above is over-read.

1. **Nothing here was measured.** This is a source read at one tag. Every performance statement is
   either quoted from `qemu_bql_spike.md` / `guest_memory_lock.md` / §6.7 or is an `[inferred]`
   consequence of code structure. §11.1 in particular argues that a *measured* number does not apply
   to QEMU; **it does not supply the QEMU number**, and that number is unmeasured.
2. **It does not establish that our device passes the lockless-IO acceptance test.** §7.9's row and
   L2-Q task 1 still stand: the spike measured a throwaway device with trivial handlers. Everything
   in §2 is about the mechanism, not about our handlers.
3. **The BAR1 named unknown is untouched.** Data-carrying aperture writes still cannot use ioeventfd
   and still stay on the vCPU inside a trap; §3's read-side finding does not reach them. The
   experiment in `qemu_bql_spike.md` §8 is unchanged and unrun.
4. **The discard question (§10) is open**, by design — the balloon/`qemu_ram_block_from_host` path
   was not read.
5. **arm64 was checked only for absence of an exclusion, not for presence of support.**
   `memory_region_enable_lockless_io` lives in generic `system/memory.c` and is honoured in generic
   `system/physmem.c`, and the KVM MMIO exit path that reaches it is generic. **[inferred]** it
   therefore applies on arm64/KVM. **No arm64 host was involved**, and upstream's own caveat that
   the flag is KVM-only (TCG ignores it) is unverified here.
6. **No claim about QEMU versions above 10.2.** The floor is a minimum; nothing was read at 10.3+ or
   at master, and the floor-ratchet residual (2) of the decision box means that is deliberate.
7. **The in-tree Rust assessment (§4) is a read of the tree layout and three files**, not an attempt
   to build an out-of-tree consumer. If someone believes a stable binding path exists, that is a
   claim to test, not to argue — and it would change row 17's verdict, not merely its wording.
8. **No `MemoryListener` / `RAMBlockNotifier` design was done.** Whether the adapter should observe
   topology changes at all — e.g. to notice a guest re-programming a BAR under us — was not
   considered, and §7.6's triggers do not currently include one.

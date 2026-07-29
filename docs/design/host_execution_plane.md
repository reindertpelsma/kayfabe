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
  assuming.
- **[unverified]** Whether the C needed a QEMU patch for any of this. It ran on 9.2.0 from
  `C: scripts/build_qemu.sh:10`.

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

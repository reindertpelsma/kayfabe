# V3 worst-trap inventory — 2026-10-22

**STATUS: RESEARCH, 2026-10-22.** W1, source base `e9c6d39e5`; one doorbell
readiness-loop fix, incomplete remediation. This is not a claim that a guest
userspace trap is bounded end to end.

CODE READ: governing rules are `THE_CONSTRAINTS.md` §§41,47,48,48.2 and
`OWNER_RULINGS.md` A.4/AI. The W1 instruction requires constant work and decline
instead of waiting. A.4's historical synchronous PRAMIN exception does not make
an RM ioctl bounded or authorize unrelated blocking locks.

CODE READ: input audits were read at race audit `fc3db5f2e`, blocking audit
`40185d178c`, and constraints audit `ce106e5204`. Their worker lock graph is
cross-referenced below; it is not new timing evidence. No GPU, guest VM, remote
host, or closed binary was used for W1.

## Reading the inventory

CODE READ: V = vCPU; Q = QEMU main/BQL; D = register drainer. U = ordinary
userspace can write an admitted mapping or induce the operation through its
kernel driver's allocation/map/free API; K = requires kernel-controlled MMIO,
PCI configuration, or reset. U/K means either. Indirect U reachability is
conservative: the exact Linux/Windows admission of every privileged RPC has
not been established. A K-only direct handler can still contend with U work.

INFERRED: a software operation count is not a wall-time guarantee. Host
preemption, memory faults, PCIe completion, and kernel scheduling have no
finite upper time bound here. BOUNDED-CONSTANT means a fixed number of
software steps with no waiting loop or blocking capability; it does not mean
that hardware has established a sub-millisecond maximum. BOUNDED-BY-GUEST-VALUE
requires an explicit cap. BLOCKING takes precedence over UNBOUNDED when both
occur. Parent handlers inherit the worst class of every selected subpath.
Rows are subpaths, not a count of mutually exclusive complete handlers.

CODE READ: rows cite functions in `qemu/hw/misc/kf3/kf3.c` (C),
`crates/kf-qemu/src/` (QEMU Rust), `crates/kf-trap/src/` (trap), and
`crates/kf-chan/src/` (channel). Lock names refer to the following lock table.
Unless stated otherwise there is no allocation, ioctl, or lock on a row.

| ID | Entry/subpath and trigger | Thread / authority | Locks, loops, allocation and host operations | Class and worst-case justification |
|---|---|---|---|---|
| T01 | CODE READ: `Device::bar0_read/write`, reset gate closed or bad FFI handle | V / U/K | `Gate::enter` atomic add/sub; no retry | BOUNDED-CONSTANT: fixed gate/return steps |
| T02 | CODE READ: BAR0 ROMD; mapped BAR1/BAR2 data and USERD | V / U/K | Direct memslot, no device callback; missing data regions fall through T15 | BOUNDED-CONSTANT: zero device-handler steps; host memory latency unbounded externally |
| T03 | CODE READ: `bar0_read_served`, shadow/FSP/hole reads | V / K; diagnostic reads U/K | Piece binary search, hole search over construction-time map, <=8 bytes; FSP cursor atomics | BOUNDED-CONSTANT: immutable host-derived region counts, C BAR0 pieces <=64 |
| T04 | CODE READ: Hopper+ memop START/COMPLETED read | V / K | Request atomic increment and one nonblocking eventfd write, or two atomic loads; host cache op is VA-thread work | BOUNDED-CONSTANT: no RM ioctl in read callback |
| T05 | CODE READ: Timer-page writes, usermode page non-doorbell writes | V / U/K | Timer early return; usermode classifier returns `None`; no queue append | BOUNDED-CONSTANT: fixed decode/drop work (optional outer diagnostics T17–19 still apply) |
| T06 | CODE READ: Unknown/dead-token doorbell | V / U/K | Token decode/load, return; no bitmap or wake for unknown route | BOUNDED-CONSTANT: fixed decode steps |
| T07 | CODE READ: Passthrough-token doorbell, diagnostics off | V / U/K | `HostRm::doorbell`: release fence, one pre-mapped volatile store; atomic ledger updates | BOUNDED-CONSTANT: fixed software work; stalled device MMIO latency needs hardware evidence |
| T08 | CODE READ: `ChanPlane::note_inline`, `KF3_MAPLOG` enabled | V / U/K | Clock and `eprintln!`; stderr mutex and `write(2)`; no per-call I/O time cap | BLOCKING: another logger or filesystem can hold up the trap |
| T09 | CODE READ: Translated/Emulated doorbell: `TokenWord::ring` | V and D / U/K | Uncapped weak-CAS loop atomically couples state and applied sequence, bitmap publish, wake bump; at most one eventfd signal after success | UNBOUNDED: no per-caller CAS attempt limit under contention or spurious failure |
| T10 | CODE READ: Privileged BAR0: `PrivRing::push`, `hole_store` | V / K | Ring reservation CAS retry; subword hole `fetch_update` retry, fixed shadow stores; ring-full poisons rather than waits | UNBOUNDED: fixed queue capacity does not bound a single CAS caller |
| T11 | CODE READ: MMU invalidate writes: `invalidate_write` / `InvalidatePort::write` | V / K; U requests can lead kernel here | Plain shadow stores, trigger `fetch_add` plus `fetch_max`, one eventfd signal; no table walk or RM ioctl here | UNBOUNDED: trigger `fetch_max` requires a separate wait-free proof; no numeric bound claimed |
| T12 | CODE READ: CPU interrupt mask/W1C/trigger and `latch_and_deliver` | V/D / K or guest work completion | Atomic bit ops; loops over host-derived leaf array; shadow stores and nonblocking irq eventfd write; hole shadow inherits T10 if selected | BOUNDED-CONSTANT: interrupt-tree operations alone, family leaf count <=16; parent includes T10 |
| T13 | CODE READ: Display PUT, cursor, event and plain writes: `DisplayPlane::trap_write` | V / K; U control of submitted display content indirect | PUT/cursor atomics and wake; event publication <=8 passes, <=8 heads, but constructs `Vec` snapshots each pass; optional trace stderr and lazy `OnceLock` initialization | BLOCKING: allocator/logger capabilities on V; finite head/pass counts do not bound them |
| T14 | CODE READ: PRAMIN base write: `pramin_write` -> `PraminPool::repoint` | V / K; U-induced mapping work can share locks | L1–L3, 16 slots, temporary Vecs, RM map/open/mmap/MAP_FIXED; outer loop repeats while another writer changes `pramin_want` | BLOCKING: RM and mutex wait have no finite bound; outer retry also UNBOUNDED |
| T15 | CODE READ: `kf3_dummy_*`, `kf3_bar12_read/write` fallback | V / U/K | Return zero / ignore; no mapping on fallback | BOUNDED-CONSTANT: fixed no-op handler |
| T16 | CODE READ: Hopper+ BAR1 overlay write: `kf3_bar1_um_write` | V / U/K | VF-relative range check then same BAR0 doorbell path T05–09; nonzero VF pages ignored | UNBOUNDED: inherits T09 for worker-served tokens; no separate inline-copy path |
| T17 | CODE READ: `kf3_dbcpl`, C trace/observer wrappers | V / U/K when host diagnostic enabled | Two KVM vCPU GET ioctls plus stderr for CPL probe (first 4096); QEMU trace backend/observer work for read trace | BLOCKING: record caps do not cap ioctl or log latency; diagnostics default off |
| T18 | CODE READ: `Bar0Trace::note` | V / U/K, default off | Atomic reservation/records, <=4096 admitted accesses; single CAS closing window, no I/O | BOUNDED-CONSTANT: one record attempt; drainer prints later |
| T19 | CODE READ: `KF3_PROF`, BAR0 census/histogram | V / U/K, default off | Two clock reads; `OffTable::add` <=4096 probes; `Hist::add` atomics including `fetch_max` | UNBOUNDED: max RMW has no portable wait-free guarantee; no per-offset max exists yet |
| T20 | CODE READ: `kf3_region_add/del` -> `ram_add/del` | Q or V+BQL / K; U shares readers | L2 write, retain/sort/push of RAM-block Vec; revoke generation; `handoff_released` L5 after mutation | BLOCKING: waits behind relay CPU copies, allocation; block-count admission cap not established |
| T21 | CODE READ: `kf3_db_section` -> `DbFast::site_add/del` | Q or V+BQL / K; U map can induce BAR1 overlay listener indirectly | L4, iterate registrations/sites, KVM_IOEVENTFD; allocations and failure logging | BLOCKING: waits behind act KVM ioctl, then performs further ioctls under BQL |
| T22 | CODE READ: `kf3_ov_bh` / `kf3_ov_apply` / overlay completion | Q+BQL / U/K map/unmap | L12 queue pop until empty; scan property-bounded overlay slots (default64); QEMU memory-region transaction/listeners/KVM memslot work; free requests; L13 completion | BLOCKING: BQL spans locks, allocator and KVM work; queue-to-empty also lacks a turn budget |
| T23 | CODE READ: `kf3_read_trap_bh` | Q+BQL / host diagnostic plus guest RPC sequence | ROMD toggles across <=64 pieces, QEMU memory transactions/listeners | BLOCKING: KVM memory topology work under BQL; default off |
| T24 | CODE READ: `kf3_vector_use/release` MSI-X routes | Q or V+BQL / K; guest completions signal irqfds | <=32 vectors; QEMU route tables, KVM routing/irqfd ioctls, allocation; no host RM | BLOCKING: fixed number of ioctls is not a bound on their latency |
| T25 | CODE READ: `kf3_msi_user` diagnostic interrupt path | Q+BQL / U/K work completion | One notifier drain, observer/trace backend, `msix_notify`; normal path uses KVM irqfd instead | BLOCKING: diagnostic trace capabilities; normal delivery is T12/eventfd |
| T26 | CODE READ: `DbFast::service_ready` readiness polling | D / U/K doorbells | After W1: one epoll_wait(0), <=64 ready tags; counter read then delivery; L6 and `sync`/sink separately T09,T27 | BOUNDED-CONSTANT: polling/delivery-count subpath only; whole call still inherits T09/T27 |
| T27 | CODE READ: `DbFast::sync` and `on_ready` command processing | D / U/K allocations/frees | L6, `try_recv` until empty; HashMap insertion/removal, eventfd final drains, ack send; missing tags call sync again | UNBOUNDED: no explicit command budget; allocator and sink can also block; not fixed by T26 |
| T28 | CODE READ: `Plane::drainer_pass` -> `Device::apply_register` -> GSP/RM statements | D / U/K kernel-mediated RPC | <=256 queued registers per pass; L7, guest queue decode/copy, per-statement Vec/HashMap allocations; `service_ready` before each register | BLOCKING: bounded register count does not bound individual RPC service or its locks |
| T29 | CODE READ: Channel alloc/free/schedule/preempt RPC handlers in `chan.rs` | D / U/K through RM APIs | L8–L11, L14–L17; slot-lock waits across worker pump/retire; pt waits across publication/notifier reads; send act requests | BLOCKING: multiple slow-holder chains; returning failure on contention would violate RPC completion semantics |
| T30 | CODE READ: GSP guest-RAM reads: `Ram::read` -> `RamMap::block_for` | D / U/K RPC buffers | L2 read; linear RAM-block scan and guest-sized bounded-by-buffer copy | BLOCKING: queued writer can make read wait behind a slow relay reader |
| T31 | CODE READ: Drainer `release_settled`, held replies / RC / hotplug / preempt delivery | D / U/K completion | L7,L8,L10,L11,L15,L17; held-reply scans, result queues, `display.model`; allocations and diagnostics | BLOCKING: no complete finite transitive hold bound; completion list sizes need admission proof |
| T32 | CODE READ: `Device::request_reset` / C reset hold | Q or V+BQL / K | Reset gate/counters and drainer eventfd only | BOUNDED-CONSTANT: request itself never waits |
| T33 | CODE READ: C reset exit -> `wait_completed` | Q+BQL / K system reset; excluded on V | Sleeps/rechecks until WAIT_BOUND=2s, `stage` takes L18; no running vCPU in system-reset caller | BLOCKING: deliberate BQL wait; stage mutex can exceed nominal timeout; not an exception for live traps |
| T34 | CODE READ: `reset.rs::poll`, drainer stage callbacks | D / K reset | L18 across sequencer stages; L7 and reset owners; per-client scans and reset-state allocations; act/VA work acknowledged later | BLOCKING: stage-local locks not proved finite; sequence is asynchronous but individual stages need proof |
| T35 | CODE READ: `kf3_gfx_update`, refresh ready/backstop, console frame/cursor callbacks | Q+BQL / guest-produced frames, host refresh | Console atomics for frame descriptors/refresh; L19 cursor snapshots, QEMU surface/cursor allocation and pixel conversion | BLOCKING: allocation/shared cursor state under BQL; worker GPU copy is separate |
| T36 | CODE READ: Broker socket/frame/timer callbacks -> `BrokerSeat::ready` | Q+BQL / guest frames plus authorized host broker | L19, packet batch READ_BATCH, nonblocking socket I/O, frame backing export/allocation and input callback delivery | BLOCKING: relay mutex and allocation; packet cap alone is insufficient |
| T37 | CODE READ: `kf3_ui_info` / display refresh request | Q+BQL / host UI influenced by guest display | Head0 bounds; dimensions clamped to16-bit, one atomic and eventfd write | BOUNDED-CONSTANT: width/height <=65535, fixed clamps and no pixel loop on request |
| T38 | CODE READ: QOM realize/exit, listeners and teardown | Q+BQL / host lifecycle, not an ordinary guest trap | Host RM initialization/cleanup, threads join, broker stop, descriptor and region frees | BLOCKING: host lifecycle only; no claim of hot-unplug bound |
| T39 | CODE READ: QEMU migration hooks | Q / host lifecycle | No VMStateDescription, pre_save, post_load or registered Rust migration FFI hook in this C device | BOUNDED-CONSTANT: zero registered device migration callbacks; migration support not implied |
| T40 | CODE READ: USER FIFO source transition / deferred API handoff | D admission then act/VA/worker / U/K via RM | L8,L9,L10,L14; `user_ring_migrate`/source rebind is worker work, not a QEMU migration hook; record can span RM operations | BLOCKING: admission/shared-lock chains on D; worker algorithm deliberately unchanged |

## Lock holders and the absence of hold-time guarantees

CODE READ: source anchors below are declarations/functions at base `e9c6d39e5`.
The race audit §3 supplies the wider holder graph; W1 re-read the direct entry
sites, PRAMIN, RAM, dbfast, PtLock and handoff implementations. A short Vec/map
operation is not a time bound: its holder can be preempted. This list names
known locks on these paths; it is not an exhaustive transitive audit of every
GSP opcode or QEMU internal lock.

| Lock ID / exact fields | Other holders and hold scope | Consequence |
|---|---|---|
| L1 `PraminPool.state`, `kf-mem/src/cpuwin.rs::repoint` | CODE READ: V repoint and VA reset; holds state across view arm/placement, host RM map and mmap; nested L2/L3 | No finite hold bound; V waits directly |
| L2 `RamMap.blocks`, `mem.rs::add/del/block_for` | CODE READ: Q/V writers; VA readers; Translated worker readers; USER worker `try_read` across CPU snapshot and guarded publication, up to MAX_SOURCE_BYTES=0xe0000 bytes | Q/V writer can wait for UC copy or preempted reader; D read can wait behind writer |
| L3 `TrapNodes.nodes`, `TrapNodes.retire`, `mem.rs::arm_store_in_trap` | CODE READ: V receiver/retire sender, replenisher/reaper queue operations; host map/mmap follows node checkout | Even brief mutex operations are not wait-free |
| L4 `DbFast.reg`, `DbFast.tx` | CODE READ: Act register/deregister; listener site_add/del. reg spans KVM_IOEVENTFD/epoll_ctl and tx; deregister's later ack wait is on act | Listener can inherit an ioctl's duration; tx queue operations allocate |
| L5 `Handoff.list`, `kf-chan/src/handoff.rs` | CODE READ: Losing workers register (Vec contains/push); every PtGuard release plus RAM/VA release takes the list when waiters exist; list released before rings | Protected M protocol: list mutex + scan/allocation; not safe to silently invoke as a bounded trap fallback |
| L6 `DbFast.side` | CODE READ: Drainer `on_ready/service_ready` only in production; held across sync and delivery; tests may also access it | No proven peer-thread wait in current production; allocation/uncapped commands remain |
| L7 `Device.gsp`, `held_stamps` | CODE READ: D FSM/apply/release/diag; startup publication Q; FSM held across guest copies and nested handler locks | No finite transitive hold bound |
| L8 `ChanPlane.pt` (`PtLock`), `pt_objs` | CODE READ: D/act blocking; USER worker try; worker `rc_scan` blocking; pt held across publication/storage writes or notifier loads; PtGuard drop calls L5 | try-lock alone does not bound release path |
| L9 `ChanPlane.slots` RwLock, each `Mutex<Slot>` | CODE READ: Worker try-lock across complete Translated pump (RM view arm and UC copies); act retire holds slot through up to200 sleeps of1ms and RM free; D schedule/free/flags take blocking lock | Direct D slow-holder wait; no finite RM bound |
| L10 `ChanPlane.user_rings`, each `Cell.inner` record, `Lease.storage` | CODE READ: Worker map lock across peeks/UC loads; record across turn; act record across deferred API alloc/release; VA storage across RM and clears | D retire and admission wait; record/storage are protected algorithm state |
| L11 `relays` map, each `Mutex<Relay>` | CODE READ: Worker serve, act drop with logging and UC loads; optional PB-peek may arm CPU view | No finite hold bound |
| L12 C `bar1_q_lock` | CODE READ: VA enqueue allocates request before locking; Q pops before applying; queue mutex itself brief | Preemption still makes Q wait; queue-to-empty lacks budget |
| L13 `Bar1Overlay.done` | CODE READ: Q completion push; VA poll/take; Vec operations | Shared blocking mutex on BQL |
| L14 `Book.pending`, user VA `Queue.pending/held`, `Regions.state`, `NativeQuota.state`, `ChanPlane.caps` | CODE READ: Act commit/cancel; D free/cancel; held queue spans storage preparation/RM; pending -> pt -> quota -> caps | Cancellation must preserve pending birth and reply lifetime; no drop-on-contention fix |
| L15 `PlacedRows` RwLock, `RowsLog.log`, `GpuMirror.sked`, mirrors/spares/RingSlots | CODE READ: VA placement/commit and act steer; worker row reads across UC snapshots; change log <=4096 entries; reset/diagnostic callers | D/VA can wait; the log's cap does not bound underlying copy/host operation |
| L16 `Inbox.q/splits/gates/split_tokens/split_results/walk_waiters` | CODE READ: D enqueue and VA take/finish; some worker access; Vec/HashMap mutation, no intentional host ioctl under these locks | Brief holder still has preemption/allocator exposure |
| L17 channel maps `by_obj/sw_objs/scopes/dbg/groups/cuda_limit/guest_sched/enc_sessions/acts/rc_queue/preempt_done/usermode_off/user_birth_holds`; `Plane.kernel_tokens`; deferred `Registry.inner`; `HostRm.objects` | CODE READ: D/act, workers for sw/acts/kernel ownership; deferred registry calls live-handle closure under lock; host objects is a witnessed leaf | Per-opcode closure/holder census incomplete; cannot certify whole drainer |
| L18 `Reset.run` | CODE READ: D poll holds across stage callback; Q `stage/status` also takes it | Nominal reset timeout is not a strict upper bound on lock acquisition |
| L19 `BrokerSeat.relay`, `BrokerSeat.console` (try-lock), `CursorShare.slot` (try-lock), console atomics; `DisplayPlane.model` | CODE READ: Q broker/console, D diagnostics/controls, display worker; relay holds across packet/input/frame processing; cursor shares decline on contention; model spans display work | Q/D wait and allocation scope needs separate display/broker audit |

## Tests-first change and protected boundaries

INFERRED: initial audit hypothesis H1 was that a guest-triggered handler can
wait behind a worker lock. Its falsifier, stated before reading the paths,
was that every reachable lock attempt declines immediately and no downstream
blocking call runs on the trap thread. CODE READ: T20,T21,T29 contain blocking
acquisitions, so that falsifier did not hold. This source finding does not
establish every ordinary-userspace admission or an elapsed stall duration.

MEASURED: at EXPECTED RED commit `6113dbcca`, the two
`dbfast::tests::worst_trap_*` tests each returned 128 deliveries against a 64-step
bound (`traces/worst_trap_20261018/results.txt`). They model legal registered
doorbell stores through real nonblocking eventfds and a fake KVM placement
verb. One uses 128 ready tokens; one signals a token immediately after delivery
drains it. Neither uses sleeps, a guest thread scheduling assumption, or a
wall-clock deadline.

CODE READ: W1 changes only the repeated readiness-poll loop in `service_ready`
to one poll. Level-triggered epoll retains the unconsumed suffix and stores
after drain. No timer, coalescing delay, GP_PUT poll, source inspection, mapping,
or relay/worker algorithm change is introduced. Command sync and callback
work still have independent open bounds. The production Sink still inherits
T09, so the whole drainer call is not certified by these tests.

MEASURED: W1-dbfast-20261022 (2026-10-22, source base `e9c6d39e5`)
full GPU-free product run finished with 3,750
passes, 1 failure, 20 ignored tests; see
`traces/worst_trap_20261018/results.txt`. The unchanged kf-mem test
`batch::tests::the_act_thread_waits_for_a_chunk_not_for_the_operation` recorded
a 169,729 us ledger-lock wait against its 60,000 us bound. No timeout or threshold
was changed, and no rerun replaced the failed result. The full suite is not
green; the changed kf-chan library's dedicated run was 202/0.

MEASURED: W1-dbfast-20261022 (2026-10-22) product all-target Clippy completed
with exit 0 and `new=0`; the claims gate also reports `new=0`. Exact commands,
scope and counts are in `traces/worst_trap_20261018/results.txt`.

INFERRED: remediation must stop at the protected protocol boundary. Replacing
`TokenWord::ring` with a failed-CAS drop loses a doorbell; overwriting its word
can corrupt the applied-sequence/route/lifetime coupling. Calling the existing
handoff registry from a trap would add L5 blocking. The lane forbids redesigning
handoff epoch/PtLock/worker behavior. Connecting `relay_dispatch::DispatchWord`
is also outside decline-only hooks: its source explicitly says no VM caller,
and production `Plane::trap_write` calls the existing `TrapPath` directly.

CODE READ: open userspace-reachable items are T08,T09,T16,T17,T19,T21–22,
T25,T27–31,T35–36,T40 (some indirect admissions remain unproved). T13 and
T11 are conservatively included for kernel-mediated user activity. They have
not received independent adversarial tests or fixes in W1. K-only direct
paths T10,T14,T20,T23–24,T33–34 also remain open. This lane is PARTIAL, and
these rows must not be interpreted as resolved because the two new tests pass.

INFERRED: owner choices, with recommended next work:

1. Authorize a separate wait-free token publication design with a durable
   worker obligation on CAS loss; preserve the applied-sequence/lifetime
   contract and prove retirement/reuse races before production wiring.
2. Keep M's handoff/PtLock algorithm unchanged in W1. Give drainer RPCs durable
   deferred reply/admission states before converting slot/pt/book locks to
   try-lock; an immediate NV_ERR or skipped free is not equivalent behavior.
3. Audit BQL topology revocation with pinned backing lifetimes before moving
   `RamMap::del` or KVM site removal asynchronous. Deferring a raw-pointer
   unregister without keeping backing alive risks use-after-unmap.
4. Move diagnostic formatting/I/O and lazy flag initialization off V in a
   separately tested diagnostic seam; default-off is not an exception to the
   userspace worst-trap rule. Retain the authorized PRAMIN behavior until its
   ordering/repoint contract has an explicit replacement.

## Default-off trap timing proposal

**STATUS: DESIGN-ONLY, 2026-10-22.** No new instrumentation code or flag in W1.

CODE READ: reuse `kf-qemu/src/prof.rs` (`KF3_PROF`, selected at realize).
`Hist` already stores max/percentiles; `Device::bar0_write` times only the
inner handler, after reset/trace preamble. `OffTable` stores per-offset count
and sum, not max. BAR0 reads, outer BAR1 overlay work and C/BQL callbacks are
not covered by that write histogram. There is no `kf-prof` crate at this base.

INFERRED: proposed handler IDs are BAR0 read, BAR0 write, BAR1 usermode write,
ioeventfd batch, RAM listener add/delete, doorbell site add/delete, MSI-X
route use/release, overlay BH step, reset request, and reset exit. Use a fixed
preallocated array of existing Hist-like counters per V thread and a separate
D/Q bank. Register each thread's slot before admitting its first guest trap;
no TLS initialization, map insertion or slot search may occur in the handler.
Start the timer at the outermost entry and end it after guard drop;
avoid counting a nested BAR0 write again in the BAR1 outer histogram. Timing
off must evaluate neither clock nor histogram, allocate nothing, and emit no
logs. Export snapshots from a diagnostic thread. A global atomic max using
CAS still needs its own structural bound; prefer single-writer thread-local
max and atomic published samples over a shared per-trap max RMW.

INFERRED: tests must precede hook implementation: (a) fake clock panics if
consulted with diagnostics off; (b) controlled ticks surround reset gate,
trace preamble, selected handler and guard drop, including early returns;
(c) distinct handlers never share a max; (d) known samples cover bucket edges
and saturation without overflow; (e) concurrent snapshot does not lock V;
(f) mutation removing outer timing and mutation dropping max must each have
two test killers. Hardware runs then report n, max, p99/p99.9, host scheduling
conditions and revision per handler. A histogram can falsify a claimed bound;
a finite sample maximum cannot prove a universal one.

## Coverage limits

CODE READ: this table covers the C device callback families, their direct Rust
entries, and the named drainer slow-holder paths. Full transitive enumeration
of every GSP opcode, QEMU-internal lock, allocation failure and OS-driver
userspace admission is NOT DONE. No baseline hardware worst time, millisecond
stall distribution, native RC containment or sub-millisecond guarantee was
established. Class counts below apply only to the 40 listed subpaths.

CODE READ: listed subpath classes after the readiness-loop fix: BLOCKING=20, BOUNDED-BY-GUEST-VALUE=0, BOUNDED-CONSTANT=14, UNBOUNDED=6 (total 40). These are inventory counts, not timing results.

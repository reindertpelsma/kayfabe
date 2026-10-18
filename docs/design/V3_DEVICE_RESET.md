# V3 — the device reset: every guest life starts on the device realize left

**STATUS: LIVE (code), 2026-10-13; ⊘ 2026-10-18: the memory stage scrubs the whole store on a guest REBOOT (§6 item 14, `docs/OWNER_RULINGS.md` §AM) — GPU-free tested only — PARTLY MEASURED ON HARDWARE (§11: Linux fast guest, binary
`c9ac4e77`, ABI 27); the bounded wait (ABI 28) and D4 are GPU-free tested only.** Branch
`claude/device-reset-20261013` (off `integration/prod-candidate-20261013` @ `342d319d9`). §8 is the
hardware plan. Measured = a GPU-free test here (named in §7) or a hardware run (§11, the coordinator's);
inferred = read from source and not executed.

⊘ *UPDATED 2026-10-13 (owner follow-up): D2's premise was wrong — QEMU pauses every vCPU around a
system reset, so the reset now completes, bounded, BEFORE the vCPUs resume (§2, §3 stage 6); D4 is
implemented (fn 47 frees the dead RM's leftover twins, §3 "Unload"); D1 and D3 are decided (§10).*

⊘ This supersedes, for kf3, the "no reset path" statements in `V3_FEATURE_GAPS.md` (H15, §3.8) and
`V3_DISPLAY.md` §4.11.11 item 4; both carry a dated note pointing here.

## 1. What was wrong, and the two requirements

`[read]` Before this branch `kf3_class_init` set only `realize`/`exit`; `GspFsm::device_reset` (E11)
had no caller; nothing in kf-qemu reacted to a QEMU reset. A guest reboot, the reset button, a
bugcheck or triple-fault reset, QMP `system_reset` or a bus reset left every piece of kayfabe state —
GSP phase and queues, host channel twins, VA mirrors, BAR views, doorbell registrations, display
engine state, the register shadow, latched interrupts — in the next guest life
(`V3_FEATURE_GAPS.md` H15: run 102's `PeerWritePtrOutOfRange` then Code 43).

Owner requirements (2026-10-13, binding):

1. **Unload → basic display.** When the guest driver unloads (`UNLOADING_GUEST_DRIVER` fn 47 /
   Booter Unload → Halted), the display goes back to the firmware GOP framebuffer.
2. **Reset → the device as realize left it**, from any state, with every host resource the guest
   owned released — a security property: a hostile guest resets itself whenever it likes.

The design basis is `THE_CONSTRAINTS.md` §40 (two lifetimes): **tier A** (the VM: the store
reservation, the host RM session, the walker's CUDA context, the BAR windows and their scratch, the
threads, the eventfds, the display's frame slots and broker connection) survives; **tier B** (one
guest driver life: GSP phase, channels, VA spaces, mappings, BAR views) is reset. The reset never
releases the store (owner, §40: "a temporarily driverless guest is not a departed guest").

## 2. QEMU's side (10.2.4, read at `/data/qemu-vfio-gsp-10.2.4`)

- kf3 is a conventional PCI device (`INTERFACE_CONVENTIONAL_PCI_DEVICE`): no FLR. It is reset by
  `qemu_system_reset` (reboot, reset button, triple fault / bugcheck with `-action reboot=reset`,
  QMP `system_reset`) and by a secondary-bus reset of a bridge above it; and **once at machine
  creation**, right after realize.
- ★ `qemu_system_reset` runs on the main loop between `pause_all_vcpus()` and `resume_all_vcpus()`
  (`system/runstate.c:868-870`: `pause_all_vcpus(); qemu_system_reset(request); resume_all_vcpus();`),
  so no vCPU is inside a trap then and **no vCPU runs until it returns**: a bounded wait anywhere in
  the reset tree blocks no running vCPU. `qemu_system_reset` → `pc_machine_reset` →
  `qemu_devices_reset` → `resettable_reset(root reset container)`: one traversal — every `enter`,
  then every `hold`, then every `exit` (`hw/core/reset.c:173-177`, `hw/i386/pc.c:1706-1721`). A bridge's bus reset runs on the vCPU that wrote the bridge's
  config space, BQL held, **other vCPUs running** — kf3's BAR0 pieces are lockless, so a vCPU can be
  inside `kf3_bar0_write` at that moment. The design handles that case (§4).
- The device's Resettable `hold` phase (`rc->phases.hold = kf3_reset_hold`, the e1000 pattern) runs
  before the bus's own hold (`pcibus_reset_hold` → `pci_do_device_reset`, `hw/pci/pci.c:539-595`),
  which clears the command register and the BARs (`pci_reset_regions` + `pci_update_mappings`: the
  BAR regions leave the address space, so kf3's memory listener sees `region_del` and Rust's
  doorbell sites go), and `msix_reset` masks every vector (`hw/pci/msix.c:560-571`), which fires
  kf3's release notifier: every KVM irqfd route is removed. Neither touches Rust's eventfds' counts.

## 3. The reset, in order

`kf3_reset_hold` → `kf3_device_reset` → `Device::request_reset`: close the vCPU gate, count the
request (and whether it came from a vCPU), signal the drainer, return. Then, in the same traversal,
`kf3_reset_exit` → `kf3_device_reset_wait` (stage 6) waits on the main loop, bounded, until the reset
completed. ⊘ *CORRECTED 2026-10-13 (owner): this section said "Nothing waits under the BQL" and let
QEMU resume the guest at once; with every vCPU paused a bounded wait blocks no vCPU, and it is what
keeps the old life's channels off the new life's RAM.* The register drainer drives
`crate::reset::Reset::poll` on every pass while a request is pending (it then applies no privileged
write, posts no held reply, delivers no RC/hotplug/preempt event, polls with a 1 ms wait instead of
parking, and keeps servicing the doorbell registry so the act thread's deregistrations are acked).
Each stage is either synchronous and bounded on the drainer or a request to the owning thread,
polled for a per-epoch acknowledgement (`reset::Ack`):

| # | stage | who | what |
|---|---|---|---|
| 0 | request | main loop / vCPU, BQL | gate closed, request counted, drainer woken |
| 1 | Quiesce | drainer | wait (by polling) until no vCPU is between gate entry and exit; then, if no guest access was admitted since the last reset began (`Reset::touched`: every admitted BAR0 write or read exit marks it), the device is **pristine** — nothing of a guest life exists — and the reset completes here without asking any owner (the machine's start-up reset; a reset straight after a reset) |
| 2 | Drainer | drainer | privileged ring discarded and poison lifted; every Translated pump stopped (`ChanPlane::stop_pumps`, `try_lock` only); GSP E11; a fresh provisional chain (`ReselectAtFn1::reset_life`); `published` map; held stamps; hotplug word; os-event arms; deferred-API registry; GSS boot cap; CPU interrupt tree; FSP EMEM |
| 3 | Release | act thread + display worker | act thread: every guest client still named by any channel-plane table is freed through `ChanPlane::free` — the guest's own client-free path (the teardown order `kf_core::lifetime::ORDER` states: tokens retired, doorbell fast path deregistered, twins/relays/USER rings/groups/debuggers/CUDA limit/encoder sessions released) — behind every act already queued; a barrier act repeats rounds (≤ 400 × 5 ms) while clients reappear or retries are parked, then clears the non-host per-life tables and acks. Display worker (once no copy is in flight): model, ports, engine, scan state, pacer, vblank gate, queued completions **dropped**, boot layer re-armed; acks |
| 4 | Memory | VA thread | queued walks/splits/held clears dropped (`VaManager::abandon_life`), a walk in flight left to land; only `Retire` statements of the old life applied; inbox split/gate/waiter/cache-op state cleared; Windows private-VA queue served (bounded); invalidate port idle (armed request **abandoned**, never completed); PRAMIN all scratch; every guest VA space retired by `retire_mirror`; our BAR1/BAR2 roots and the boot framebuffer `[0, G)` zeroed on the GPU; K_BAR1/K_BAR2 re-registered with fresh walker slots (`lifetime::Step::ResetWalkerState`), windows back to the boot view (`CpuWindow::reset_to_boot`: scratch first, then each view's aperture; BAR1's seed placed again; Hopper+ doorbell views removed through the same FIFO overlay verb); acks |
| 5 | Finish | drainer | token sweep (every token word fresh, rung bitmap clear) — **pending while a worker owns a token** (nothing else runs until the sweep is complete); interrupt tree and hotplug word reset **again** (anything latched while the owners finished is the old life's); BAR0 shadow rewritten word by word, only where it differs from the image realize wrote (`piece_image`; never a zero pass the running vCPUs could read), hole pages rebuilt, GSP registers / display static words and events / interrupt read-back republished; stale interrupt counts drained from every vector's eventfd; gate opened |
| 6 | wait (exit phase) | main loop, BQL, vCPUs paused | `Reset::wait_completed(newest request, reset::WAIT_BOUND = 2 s)`: returns when stages 1-5 completed — the channel plane EMPTY, the memory plane reset, the gate open — so the vCPUs resume on a device as realize left it. Past the bound: one loud line naming the stage (`NOT complete after 2s … the old life's host channels may still run` before Release completed), `waits_timed_out` counted, and the reset completes in the background with the gate closed (the ABI 27 behaviour). On a vCPU thread (a bridge's bus reset; the other vCPUs run) it never waits |

**None of the threads stage 6 waits for needs the BQL** (the main loop holds it): the drainer and
the act, VA and display threads call into C only through `qemu_bh_schedule` (the BAR1 overlay verb,
the diagnostic read trap — a schedule, never a wait for the bottom half) and `kvm_vm_ioctl`
(`KVM_IOEVENTFD` for the doorbell fast path), which takes no QEMU lock: its `accel_ioctl_begin`
only waits on a memslot inhibit (`accel/accel-blocker.c:43-51`), begun and ended inside one main-loop
memory commit, never across this wait. The VA thread's BAR1 overlay completions arrive by bottom
half, but no reset stage waits for them (the reset abandons deferred clears). The display worker's
frames are taken by the main loop, never waited for by the worker. Everything else they wait on is
the host GPU or host RM. If a stage does stall (a host ioctl that hangs), the bound — not a lock —
ends the wait. [inferred: read, not run]

A request during a run restarts at stage 1 with the gate closed (every stage is idempotent); since
the gate was closed all along, the rerun finds the device pristine and completes at once. Several
requests before a poll are one run. The stage is mirrored in an atomic so the status line and the
stall report never take the sequence's lock (the review's HIGH finding: they did, and the drainer
deadlocked exactly when a stage stalled). An owner that is not running (`Device::va_up` / `display_up`,
set on loop entry, cleared on any exit) is not waited for and its epoch is marked done, so it owes
nothing when it starts (the start-up reset can run before the display worker's first pass).

**Unload (requirement 1).** fn 47 without `bInPMTransition`, with `gop=on`, already re-seeded BAR1
`[0, G)` (B5, `V3_DISPLAY.md` §4.11.13). It now also calls `DisplayPlane::rm_unloaded`: the display
worker ends the display's guest life exactly as in stage 3 and `choose_shown` picks the boot layer
(store `[0, G)`) — what efifb/simpledrm, the firmware console or Windows' basic display adapter draw
through BAR1 `[0, G)`. Derivation: GSP-RM's own unload destroys every display object it held, and
`kbusStatePreUnload` → `kbusTeardownMailbox` returns BAR1 to physical mode
(`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:746-787`); CPU-RM's
`kdispStateUnload` restores nothing on the display (`kern_disp.c:679-692`), so what the monitor shows
after an unload is whatever reaches the scanned surface — the boot framebuffer once BAR1 is physical.
⊘ This is the owner's rule, not bare metal's: on bare metal a teardown that never sent
`PRESERVE_HW` shuts the heads and the screen is black (`V3_DISPLAY.md` §4.11.13 arm (b)). With
`gop=off` (no boot layer, no seed) the display is unchanged at unload — owner decision D3: there is
no boot framebuffer to return to.

**Unload frees the dead RM's twins (owner decision D4, 2026-10-13).** The same fn 47 (no PM
transition, no GC6 entry — `bar1phys::unload_actions`), whatever `gop`, frees every client the
channel plane still holds (`ChanPlane::free_dead_clients`): GSP-RM's own unload destroyed every
object it held, so those twins are dead. Each client is freed through `ChanPlane::free` on the
drainer, as the statement the guest's own client free would be — host acts on the act thread,
nothing waits, no completion forged. The set is taken at fn 47 (every birth of the dead RM completed
before it: its RPCs are synchronous) and nothing sweeps later arrivals, because there is no gate here
and a reloading guest may already be allocating its next life; the reset's barrier is therefore not
used. A clean unload has freed everything already and finds nothing. ⊘ The memory plane is NOT
reset at fn 47 (only BAR1's seed, as B5): a mirror the dead RM never retired stays until a device
reset — named open item.

## 4. Device state, owner by owner

Lifetime A = realize-time, survives; B = one guest life, reset. "Race" says how in-flight work is
excluded. M = covered by a GPU-free test (§7), I = inferred from reading.

| state (owner) | life | reset action | threads that touch it | race excluded by | M/I |
|---|---|---|---|---|---|
| vCPU write paths: `bar0_write`, BAR1 usermode views, `bar0_read` hole exits | — | gate closed: write dropped (counted), read answers all-ones | vCPU | the gate: SeqCst add on entry, stage 1 waits (polls) for zero inside | M (gate stress test) |
| `PrivRing` (plane) | B | `discard_all` (seq stays monotonic, poison lifted) | vCPU push, drainer | gate quiet before stage 2 | M |
| `GspFsm` | B (system-info cell A) | E11 | drainer (GSP lock) | drainer-only | M (11 tests) |
| served chain (`ReselectAtFn1`: object graph, channel/VA/display rows, guards) | B | `reset_life`: fresh build for the realize-time table | drainer | drainer-only | M |
| `Gsp.published`, `held_stamps`, `hotplug_pending` | B | cleared (hotplug again at Finish) | drainer; display worker sets hotplug | Finish runs after the worker acked | I |
| `ChainLogs.os_events` (registrations, non-stall arms) | B | `clear` (counters kept) | drainer, workers read arms | arms zero before channels are freed: no paced raise | I |
| unserviced / fault-buffer logs, census, refusal ledgers, counters | A (diagnostic) | kept: the VM's run, never a decision input | — | — | I |
| `defapi::Registry` | B | `clear_life` | drainer (chain), act/worker (lookups) | channels gone at stage 3 | M |
| `gssnative::Stats` (experiment) | B (cap) | `new_boot` (cap only) | act thread | — | I |
| `CpuIntr` | B | `reset` at stage 2 and Finish, read-back republished | vCPU, workers (latch), display | gate + second reset after every owner acked | M |
| `FspEmem` (Hopper+) | B | `reset` | vCPU | gate | M |
| BAR0 shadow pieces, hole pages | B | rewritten where they differ from `piece_image` (the realize image) + republish | vCPU, drainer, VA, display | gate; publication is drainer-owned | I (fill shared by construction) |
| token words, rung bitmap, kernel-token set (plane) | B | retired by the frees; swept fresh at Finish (`reset_tokens`) | vCPU ring, workers claim, act retire | gate; BUSY words waited out by the free path | M |
| `VmCaps` (channel budget) | B counts | released by the frees, **not zeroed**: a count left is a host channel still alive (leak, counted) | act | — | I |
| Passthrough twins, Translated slots, relays, USER rings, host TSGs, debuggers, CUDA limit, encoder sessions (`ChanPlane`) | B (host objects) | reset: freed through `ChanPlane::free(c, c)` per guest client on the act thread, `LiveCensus` logged; fn 47 teardown (D4): the same frees issued on the drainer (`free_dead_clients`) | act, drainer (statements), workers (pump) | frees run behind queued acts (a birth in flight lands first, then is freed); the free path stops pumps and waits out BUSY tokens | I (needs a host) |
| `pt_objs`, `by_obj`, `scopes`, `sw_objs`, `guest_sched`, `rc_queue`, `preempt_done`, `pt_fifo_owed`, ledgers, completion bitset | B | cleared at the barrier | act, drainer, workers | after every free act ran | I |
| `DbFast` registrations (KVM ioeventfds) | B | deregistered by each twin's free; sites follow QEMU's BAR mapping | act (assign), drainer (ack), main loop (sites) | drainer keeps acking while the reset is pending; deliveries during a reset are absorbed | I |
| act queue's parked retries | B | barrier waits (≤ 2 s) | act | barrier re-queued with a delay, never a sleep | I |
| `Inbox` (statements, splits, gates, results, waiters, cache-op counters, walk flag) | B | stage 4 (`reset_life`; statements: only `Retire` applied) | drainer push, workers, vCPU (cache ops), VA | gate; drainer pushes nothing while resetting | M |
| `VaManager` pending/awaiting/splits | B | `abandon_life` | VA | VA-only; in-flight walk left to land | M |
| guest VA spaces (mirrors, rows, host VA spaces) | B | `retire_mirror` (unmap rows; recycle as spare or free) | VA, workers read | channels freed first (a live mirror is KEPT and named) | I |
| BAR1/BAR2 windows (`CpuWindow`), seed | B (window A) | `reset_to_boot` + fresh walker slot + roots | VA | VA-only | M |
| Hopper+ BAR1 doorbell views (C aliases) | B | removal queued through the FIFO verb | VA, main loop | FIFO: after any install still queued | I |
| our roots, boot framebuffer `[0, G)` | B | zeroed on the GPU (as at realize) | VA (walker) | VA-only | I |
| invalidate port / trigger | B | `reset`: idle, armed request abandoned (counted as completed, never cleared on the guest's behalf) | vCPU arms, VA completes | gate; a late completion finds it idle | M |
| PRAMIN pool / window word | B | all scratch, word 0 | vCPU (sanctioned syscall), VA at reset | gate | I |
| Windows private-VA queue (`userva`) | B | served (bounded) before mirrors retire | VA | — | I |
| T-space ring slots | B accounting | returned by the channel frees; leaks counted (`slots_leaked`) | act | — | I |
| display model, ports, engine, scan, pacer, vblank gate, queued completions | B | worker-side life end (§3); completions dropped | drainer (chain), vCPU (ports), worker | worker resets when no copy is in flight; gate | M (model, ports, engine, choose_shown) |
| display frames, VRAM slots, broker seat, console wiring, `failed` | A | kept | worker, main loop | — | I |
| MSI-X routes (C) | QEMU | `msix_reset` releases them | main loop | QEMU | I |
| vector eventfd counts | B | drained at Finish | any raiser | after every owner acked | I |
| store reservation, host session, walker context, windows/scratch, threads, eventfds | A | kept | — | — | I |
| guest VRAM contents other than `[0, G)` and our roots | (guest data) | **kept** (owner decision D1, §10) | — | — | I |

## 5. Accounting (the leak check)

`ChanPlane::live_census` (twins, translated, relays, USER rings, groups, debuggers, channel caps,
non-fresh tokens, doorbell registrations) is logged at every reset's barrier as
`act device reset: barrier: … channel plane EMPTY|NOT EMPTY … twins=… caps=… db_regs=…`; the memory
line logs `N VA space(s) retired, K KEPT, mirrors left M` and each BAR window's removed/kept counts.
After any reset every count must be 0 (`EMPTY`, `KEPT 0`, `mirrors left 0`, `kept 0`); `NOT EMPTY`
increments `reset_incomplete` (status line `reset[…]`). ⊘ `KF3_DBFAST_PROBE` (a measurement
registration) makes every reset report `NOT EMPTY`.

## 6. Failure modes (and what the device does)

1. **An owner never acknowledges** (an act stuck in a host ioctl, a walk that never lands): the gate
   stays closed — fail closed, the device accepts nothing — and the stall is logged once per stage
   after 5 s. A further reset request coalesces. [inferred]
2. **A host free is refused**: the guest's tables are cleared regardless (nothing of the old life is
   routable), the host object leaks and is counted (`NOT EMPTY`, `reset_incomplete`, a KEPT mirror).
3. **GPU work in flight at the reset** (a crashed guest's channels): it runs until the host's own free
   preempts and unbinds the channel (stage 3). No completion is written for it.
   ⊘ *CORRECTED 2026-10-13 (owner): the text below the line said QEMU resumes the guest as soon as
   the hold phase returns, leaving a window of milliseconds and an owner decision (D2). Wrong
   premise: the vCPUs stay paused until the reset tree returns (`system/runstate.c:868-870`), and
   stage 6 now waits there. For a system reset the window is closed — the vCPUs resume only after
   the channel plane is EMPTY — unless the 2 s bound passes (then it is said loudly, failure mode 12)
   or the reset came from a vCPU (a bridge's bus reset: no wait, the window stays).*
   ~~QEMU resumes the guest as soon as the hold phase returns; until stage 3 frees the twins
   (inferred: milliseconds), a host channel can still read the GPFIFO and write semaphores in guest
   RAM that the next life's firmware is starting to use, and an act or a USERD relay of the old life
   that runs before its channel's free can still write its relay words there.~~ [inferred]
4. **Guest BAR0 writes during the reset** are dropped; BAR0 reads come from the shadow and show the
   old life's values until Finish (reads never exit). ⊘ *2026-10-13: with stage 6 this applies only
   to a bus reset from a vCPU or a wait past its bound — after a system reset the vCPUs resume on the
   finished device.* Firmware does not touch kf3's BAR0 (kf-gop draws
   only through BAR1); the guest driver loads seconds later. [inferred]
5. **BAR1 `[0, G)` before stage 4** shows the old life's placement (or scratch): firmware drawing in
   that window goes there and is lost; the seed is placed again at stage 4. ⊘ *2026-10-13: as 4 —
   after a system reset no firmware instruction runs before stage 4.* [inferred]
6. **A guest that resets in a loop**: each run is bounded; lines are bounded (`reset::loud`: the first
   16, then powers of two). [measured: the bound]
7. **Display copy in flight**: the worker resets after its completion (polled). [inferred]
8. **Start-up**: the reset runs once at machine creation (the hold phase). No guest access has been
   admitted yet, so it is pristine and completes in the drainer's next pass without asking any owner
   (before this, it waited for the VA thread, which can be inside a cold-box prewarm step for
   seconds while the guest already runs — the review's MEDIUM finding). ⊘ Consequence: a normal
   boot no longer exercises the full sequence; only §8's lanes do. [measured: the pristine path]
9. **The gate's cost**: one SeqCst add and subtract (plus one relaxed load) per trapped BAR0 access,
   on a cache line every vCPU shares — beside a VM exit (≈µs nested) it is small, but it is real and
   unmeasured.
10. **A fast driver reload after an unload** (inferred, small): the display worker ends the display's
   life after any copy in flight; a reload that allocates display objects within those milliseconds
   could have them dropped. RM init takes seconds. Whether Windows sends fn 47 without a PM
   transition during a TDR recovery is unknown (closed driver): if it does, the boot layer shows
   during the recovery — L5 checks it.
11. **The display worker, the act thread and USERD relays keep running the old life** for the
   moments before their reset point — with every vCPU paused (stage 6), so nothing of the new life
   is there to be touched; only a vCPU-initiated bus reset or a wait past its bound leaves it open.
12. **Stage 6 past its bound** (a host free or walk that hangs for 2 s): the main loop stops waiting,
   says so in one line naming the stage and whether the channels were already freed, and the reset
   completes in the background with the gate closed (failure modes 1, 3-5 then apply). The VM is
   never held longer than the bound. [measured: the timeout path, `the_bounded_wait_gives_up_at_its_bound_and_the_reset_completes_later`]
13. **fn 47 frees while the guest reloads**: the client set AND each free's twins are taken at fn 47
   on the drainer (statement time), so a next-life twin, born later, is never among them even if it
   reuses a dead client's handle; the free acts queue in statement order ahead of any next-life
   birth. [inferred]

14. **The reboot scrub is refused (2026-10-18).** A system reset from the main loop is a NEW OS over a store
   the old one dirtied; the memory stage zeroes the whole store on the GPU and awaits the completion
   (`crate::scrub`, `crate::memreset`). If the host refuses a chunk or the completion: the reset is **not
   acknowledged** (`life_reset.done` is reached only when every step completed), so the reset never reaches
   Finish and the gate stays closed; one loud line names the range (bounded), and **only the scrub is
   retried**, with a 50 ms..1 s back-off — never the retire, the root zero or the boot zero. The prepare step
   (mirrors retired, every BAR1/BAR2 CPU view sunk to scratch) ran first, so the guest never has a CPU view of
   the half-zeroed store. At the main loop's 2 s bound (failure mode 12, UNCHANGED contract) the vCPUs resume
   with BAR0 gated and the stage keeps retrying in the background; a guest that boots meanwhile sees a dead
   device, not an unscrubbed store. [GPU-free tested: `memreset::tests`, `scrub::tests`]

## 7. GPU-free tests (all passing at this branch's head)

Command (disk-saving flags used here because `/data` was full):
`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p kf-qemu -p kf-gsp -p kf-core -p kf-trap -p kf-mem -p kf-disp -p kf-rm`.

- `kf-qemu` `reset::tests`: `the_bounded_wait_returns_once_the_drainer_completed_the_reset`,
  `the_bounded_wait_gives_up_at_its_bound_and_the_reset_completes_later` (both with the drainer on
  its own thread), `a_pristine_device_resets_at_once_without_asking_any_owner`,
  `a_stall_report_may_read_the_status_without_deadlocking`,
  `finish_that_is_not_ready_keeps_the_gate_closed_until_it_is`,
  `an_open_gate_admits_and_counts_the_entry_until_it_leaves`,
  `a_closed_gate_drops_the_entry_and_counts_it`,
  `closing_does_not_wait_for_an_entry_in_progress_and_quiet_says_when_it_left`,
  `once_closed_and_quiet_no_vcpu_is_inside_under_contention` (4 vCPU threads × 20 runs),
  `with_no_request_the_drainer_does_nothing`,
  `a_reset_runs_every_stage_in_order_and_opens_the_gate_only_at_the_end`,
  `a_vcpu_inside_holds_the_reset_at_quiesce_and_nothing_is_touched`,
  `a_reset_immediately_after_a_reset_runs_again_and_covers_both`,
  `a_request_during_any_stage_restarts_the_whole_sequence_with_the_gate_closed`,
  `a_request_made_inside_a_stage_is_not_lost`,
  `a_stage_that_never_acknowledges_keeps_the_gate_closed_and_is_said_once`,
  `n_reset_cycles_release_everything_every_life_acquired` (50 lives, released == acquired),
  `reset_lines_are_bounded_for_a_guest_that_resets_in_a_loop`,
  `an_ack_is_per_epoch_and_a_late_one_never_satisfies_a_newer_request`.
- `kf-qemu` `chan::reset_tests::the_reset_barrier_frees_again_waits_for_parked_retries_and_is_bounded`;
  `bar1phys::tests::a_teardown_frees_the_dead_rms_twins_and_a_pm_transition_touches_nothing` (D4/D3);
  `mem::tests::a_reset_inbox_holds_nothing_of_the_old_life_and_tickets_never_repeat`;
  `display::tests::a_guest_life_that_ends_shows_the_boot_layer_again_from_every_state`.
- `kf-gsp` `boot::device_reset_from_every_phase`: `every_driver_reaches_its_phase_with_the_live_state_it_claims`,
  `a_device_reset_from_every_phase_is_e11_and_a_fresh_device`,
  `a_device_reset_from_every_phase_keeps_the_system_info_cell_and_nothing_else`,
  `a_life_ended_by_booter_unload_resets_to_a_fresh_device`, `a_reset_value_is_fresh_field_by_field`,
  `a_reset_straight_after_a_reset_is_e11_again_and_still_fresh`,
  `a_reset_of_a_device_nothing_ever_touched_changes_nothing`,
  `a_second_life_after_a_reset_is_a_first_life_on_the_falcon_regime`,
  `a_second_life_after_a_reset_is_a_first_life_on_the_fsp_regime`,
  `a_second_life_after_a_reset_is_a_first_life_with_a_system_info_cell`,
  `fifty_boot_and_reset_cycles_leave_nothing_behind`.
- `kf-trap`: `a_reset_ring_drops_the_old_lifes_writes_lifts_the_poison_and_never_rewinds_its_sequence`,
  `cpuintr::tests::a_reset_tree_is_the_fresh_tree_and_an_old_latch_never_fires_in_the_next_life`,
  `fspemem::tests::a_reset_channel_reads_as_a_fresh_one`,
  `mmuinval::tests::a_reset_port_reads_idle_and_a_late_completion_of_the_old_request_is_harmless`.
- `kf-core`: `a_reset_plane_routes_nothing_of_the_old_life_and_allocates_like_a_fresh_one`.
- `kf-mem`: `cpuwin::tests::a_reset_window_is_the_seeded_window_realize_left_and_every_view_is_released`,
  `cpuwin::tests::a_reset_keeps_a_view_whose_scratch_is_refused_and_says_so`,
  `cpuwin::tests::n_lives_and_resets_leave_exactly_the_seed_armed` (50 lives, host apertures),
  `vasmgr::tests::a_reset_drops_queued_walks_lets_the_one_in_flight_land_and_reslots_the_bar_object`.
- `kf-disp`: `model::tests::a_reset_model_answers_the_next_life_as_a_fresh_one`,
  `engine::tests::a_reset_engine_arms_nothing_and_lights_the_next_life_like_a_fresh_one`.
- `kf-rm`: `a_reset_chain_is_a_fresh_provisional_build_and_the_next_life_reselects_again`,
  `defapi::tests::a_cleared_registry_holds_nothing_of_the_old_life_and_admits_the_next_one`.
- `kf-qemu` `tests/wire_mirror.rs` (unchanged test): `kf3_device_reset` is declared identically on
  both sides of the seam. `kf3.c` was syntax- and type-checked with `-Werror` against the configured
  QEMU 10.2.4 build tree; it was **not linked or run**.

⊘ What the tests cannot show: a `Device` and a `ChanPlane` need a host GPU (`HostRm` is concrete), so
the composition — the act thread's frees on a real host, the VA thread's retire on real mirrors, the
drainer driving all of it, the C hold phase — is covered only by the coordinator's fake owners and
by §8.

## 8. Hardware test plan (strictly serial, one box; NOT run)

Preconditions: `scripts/bench/v3_gates.sh` 9/9 → `build_kf3.sh` at this branch's revision (ABI 27)
→ rebuild the raw client and the fast guest. Every QEMU log line cited is `kf3:`-prefixed. Each step
states its falsifier; record the result either way.

**L0 — start-up reset (every boot).** Boot the fast guest once.
Expect `device reset #1 COMPLETE` and `device reset #1 complete before the vCPUs resume (the main loop
waited N us)`, the heartbeat's `reset[req=1 (main=1 vcpu=0) done=1 pristine=1 waits_timed_out=0 …]`,
no stage line (pristine), then a normal boot. *Falsifier:* a stage line at start-up
(something touched the device before the machine reset), a `WAITING at` line, or the guest's GSP
boot failing where `master` boots.

**L1 — Linux, QMP `system_reset` while Running, ×5.** Fast guest (or the display lane VM with
`gop=on,display=on`), run a CUDA workload (`cuda_ladder.sh` rung 1) in a loop, then
`{"execute":"system_reset"}` on the QMP socket; wait for the guest to boot; repeat 5×. ★ ABI 28:
each full reset must log `complete before the vCPUs resume (the main loop waited N us)` with N well
under 2 s, the barrier line BEFORE it, and `waits_timed_out=0`; record N (it is the guest's reboot
pause). *Extra falsifier:* a `NOT complete after 2s` line; a host Xid between the reset and the next
boot (the old channels must be gone before the guest runs). Also once with
`system_reset` issued twice within 10 ms (expect one full run and `pristine` +1).
Expect per reset: `vCPUs out … GSP Running -> Cold (E11, N held …)`, a barrier line with `twins=0
translated=0 relays=0 … caps=0 db_regs=0` and `EMPTY`, the memory line with `0 KEPT`, `mirrors left 0`,
`BAR1: … seed Some(Placed …)` (gop=on) / `NoBootFramebuffer` (gop=off), `COMPLETE`; the guest boots
and `nvidia-smi` + the CUDA rung pass each time. Host side: `nvidia-smi` on the host shows kayfabe's
channel/memory use back to the post-boot baseline after each reset (record the numbers).
*Falsifiers:* `NOT EMPTY`/`KEPT`/`reset_incomplete>0`; a second boot failing (`PeerWritePtrOutOfRange`,
`SeqNumGap`, Code 43-like RM init failure); host memory growing per cycle; any host Xid other than
on the freed twins in the reset window (count them — failure mode 3).

**L2 — Linux `reboot` (clean) ×3, then `reboot -f` ×3, then `echo b > /proc/sysrq-trigger` ×2**
(no clean unload). Same expectations as L1; for `reboot` the barrier should report 0 clients freed
(the guest freed them), for `-f`/sysrq it frees them. *Falsifier:* as L1.

**L3 — Linux driver unload/reload.** ★ D4: with a CUDA process killed by `kill -9` just before
`rmmod` (or the driver unloaded under a held fd where the module allows it), expect at fn 47
`the guest RM unloaded with N client(s) still holding host objects … freed`, and on a clean unload no
such line; host `nvidia-smi` channel/memory back to the post-boot baseline after each cycle. `rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia`, wait,
`modprobe nvidia` + `nvidia-smi`, ×5, with `gop=on`; run `scripts/bench/display/lane.sh` with
`unload_hook.sh` (B5 arms). Expect at each fn 47: `display: … life ENDED (its RM unloaded) … boot
layer is shown again`, the console (QMP `screendump`) showing the text console/efifb content, not
black, for arm (b) as well; the reload boots. *Falsifier:* a black screendump after the unload in any
arm with `gop=on`; a reload failure; B5's existing verdict regressing in (a)/(a2)/(c).

**L4 — the fast suite after resets.** In one QEMU: boot, 3× QMP `system_reset`, then
`KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180`. *Falsifier:* below 30/30 where the same
revision without resets passes 30/30.

**L5 — Windows (signed GOP, display lane).** (a) Start → Restart ×3; (b) QMP `system_reset` while the
desktop runs ×3 (hard reset); (c) Device Manager → NVIDIA adapter → Disable, check the Basic Display
Adapter picture on the console, Enable, ×3; (d) a bugcheck (`NotMyFault` or `.crash` via the kernel
debugger) with the default auto-reboot, ×2. Expect after each: the desktop and `nvidia-smi` come back
in the SAME QEMU; for (c) the console shows the basic-display desktop (not black) while disabled; the
reset lines as in L1. *Falsifiers:* Code 43 / no NVIDIA adapter after a restart; a black console
while disabled; `NOT EMPTY`/`KEPT`; a TDR or 4101 in the first app run after a reset. Also record
whether any TDR recovery logs `fn 47 … bInPMTransition=false` (failure mode 10).

**L6 — races (stress, Linux).** QMP `system_reset` issued (i) during the GSP boot (within 2 s of the
driver load), (ii) during a CUDA run with doorbell-ioeventfd on, (iii) twice within 10 ms, (iv) during
`rmmod`. *Falsifier:* a `WAITING at` line, a hung drainer heartbeat, a second life that fails.

## 9. Review (2026-10-13, an independent read of the branch at `74243aab`)

Folded in: the stall report deadlocked the drainer (HIGH; the stage is now an atomic, test
`a_stall_report_may_read_the_status_without_deadlocking`); the start-up reset waited for a prewarming
VA thread with the gate closed (MEDIUM; pristine fast path); the shadow restore zeroed before
refilling (MEDIUM; word-diff rewrite); Finish ignored BUSY tokens (MEDIUM; Finish pending until the
sweep completes); the BAR1 physical-view counter survived the reset (LOW; zeroed); a poisoned
sequence lock stalled silently (LOW; recovered). The barrier's self-counted parked act had been found
and fixed just before (`cda43759`). Open, accepted or the owner's: whole-store scrub (D1), the
window of failure mode 3 (D2), the gate's per-access cost (failure mode 9), the reload race (10).

## 10. Decisions (owner, 2026-10-13)

- **D1 — guest VRAM across a reset: NOT scrubbed** (decided). It is the same VM's data, and real GPUs
  keep VRAM across a reset. The reset zeroes only our two roots and the boot framebuffer `[0, G)`,
  as realize does.
- **D2 — the DMA window: removed** (resolved by design). Its premise — that QEMU resumes the vCPUs as
  soon as the hold phase returns — was wrong (`system/runstate.c:868-870`); stage 6 waits, bounded,
  with every vCPU paused.
- **D3 — unload with `gop=off`: unchanged** (decided). There is no boot framebuffer to return to; the
  display stays as B5 measured (a preserved console, or black).
- **D4 — leftover twins at fn 47: freed** (decided, implemented — §3 "Unload"). The memory plane is
  still reset only by a device reset (open item: mirrors the dead RM never retired).

## 11. Hardware results so far (the coordinator's runs, 2026-10-13)

Linux fast guest, binary `c9ac4e77` (this branch at ABI 27 — before the bounded wait and D4), host
stock 595.91.07. `[measured]` unless marked:

- QMP `system_reset` ×5 mid-boot, and ×5 at 9-19.5 s into the arms (concurrent-fuzz, map-stress,
  ce-client, ce-client-guest-ram, concurrency, guest-ram-pin). Every reset logged `device reset #N`
  with the vCPUs-out gate line, `GSP Running -> Cold (E11)`, `act device reset: free every guest client`
  (2 clients early, 3 — a CUDA client among them — late), `channel plane EMPTY after 1 round`, and the
  memory plane retiring 4 VA spaces and removing 79-87 BAR2 placements.
- After each reset the guest rebooted; the final boot passed every arm (`FAST_VERDICT=PASS`); the host
  Xid count was unchanged.
- Windows LTSC (in progress): a hard reset at the signed-in desktop logged `13 Translated pump(s)
  stopped` and `GSP Running -> Cold`. Outcome of the restarted desktop: not yet reported.
- **Two resets per `system_reset`, the odd-numbered one pristine** (`[measured]`: the pattern; the
  cause is `[inferred]`). The start-up reset is #1 (pristine: nothing has touched the device). Each
  `system_reset` then shows a full reset (#2, #4, …) followed by a pristine one (#3, #5, …) that
  completes at once. QEMU's own reset tree resets kf3 once per `qemu_system_reset` (one traversal,
  §2), so the second request is a SECOND `qemu_system_reset`. The fast guest boots SeaBIOS
  (`-kernel`, no `-no-reboot`), and SeaBIOS, entered after a reset with its shadowed image already in
  RAM, takes its resume path and requests a hard reboot ("Attempting a hard reboot") — a second
  platform reset before the guest touches kf3, hence pristine. (`[read]` the strings "In resume
  (status=%d)" and "Attempting a hard reboot" are in this machine's SeaBIOS image,
  `/usr/share/seabios/bios-256k.bin`; that this path is what runs after a QEMU reset is recalled,
  not read in its source, and the bench's QEMU ships its own SeaBIOS build.) *Falsifiers:* QMP shows ONE `RESET`
  event per `system_reset` (then the second request has another source); kf3's `reset[… (main=N
  vcpu=M) …]` counts a vCPU request (a bus reset, not a platform reset; ABI 28 logs the source); a
  SeaBIOS debug log (`-chardev …,id=seabios -device isa-debugcon,iobase=0x402,chardev=seabios`)
  without "Attempting a hard reboot" between the two. Either way the pristine reset is correct and
  free: nothing of a guest life exists to undo.
- Not yet measured: ABI 28's bounded wait (its `complete before the vCPUs resume` line and the wait
  time), D4 at an unclean unload, L2-L6.

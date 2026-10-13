# V3 — the device reset: every guest life starts on the device realize left

**STATUS: LIVE (code), 2026-10-13 — GPU-FREE TESTED ONLY; HARDWARE-UNVERIFIED.** Branch
`claude/device-reset-20261013` (off `integration/prod-candidate-20261013` @ `342d319d9`). Nothing in
this document has run on a GPU, in QEMU or in a guest. §8 is the hardware plan that has to run before
any of it is called working. Measured = a GPU-free test here (named in §7); inferred = read from
source and not executed.

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
- `qemu_system_reset` runs on the main loop after `pause_all_vcpus()` (`system/runstate.c:868`), so
  no vCPU is inside a trap then. A bridge's bus reset runs on the vCPU that wrote the bridge's
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
request, signal the drainer, return. **Nothing waits under the BQL.** The register drainer drives
`crate::reset::Reset::poll` on every pass while a request is pending (it then applies no privileged
write, posts no held reply, delivers no RC/hotplug/preempt event, polls with a 1 ms wait instead of
parking, and keeps servicing the doorbell registry so the act thread's deregistrations are acked).
Each stage is either synchronous and bounded on the drainer or a request to the owning thread,
polled for a per-epoch acknowledgement (`reset::Ack`):

| # | stage | who | what |
|---|---|---|---|
| 0 | request | main loop / vCPU, BQL | gate closed, request counted, drainer woken |
| 1 | Quiesce | drainer | wait (by polling) until no vCPU is between gate entry and exit |
| 2 | Drainer | drainer | privileged ring discarded and poison lifted; GSP E11; a fresh provisional chain (`ReselectAtFn1::reset_life`); `published` map; held stamps; hotplug word; os-event arms; deferred-API registry; GSS boot cap; CPU interrupt tree; FSP EMEM |
| 3 | Release | act thread + display worker | act thread: every guest client still named by any channel-plane table is freed through `ChanPlane::free` — the guest's own client-free path (§9 order: tokens retired, doorbell fast path deregistered, twins/relays/USER rings/groups/debuggers/CUDA limit/encoder sessions released) — behind every act already queued; a barrier act repeats rounds (≤ 400 × 5 ms) while clients reappear or retries are parked, then clears the non-host per-life tables and acks. Display worker (once no copy is in flight): model, ports, engine, scan state, pacer, vblank gate, queued completions **dropped**, boot layer re-armed; acks |
| 4 | Memory | VA thread | queued walks/splits/held clears dropped (`VaManager::abandon_life`), a walk in flight left to land; only `Retire` statements of the old life applied; inbox split/gate/waiter/cache-op state cleared; Windows private-VA queue served (bounded); invalidate port idle (armed request **abandoned**, never completed); PRAMIN all scratch; every guest VA space retired by `retire_mirror`; our BAR1/BAR2 roots and the boot framebuffer `[0, G)` zeroed on the GPU; K_BAR1/K_BAR2 re-registered with fresh walker slots (§9 ResetWalkerState), windows back to the boot view (`CpuWindow::reset_to_boot`: scratch first, then each view's aperture; BAR1's seed placed again; Hopper+ doorbell views removed through the same FIFO overlay verb); acks |
| 5 | Finish | drainer | token sweep (every token word fresh, rung bitmap clear); interrupt tree and hotplug word reset **again** (anything latched while the owners finished is the old life's); BAR0 shadow refilled by the same function realize used (`fill_piece`), hole pages rebuilt, GSP registers / display static words and events / interrupt read-back republished; stale interrupt counts drained from every vector's eventfd; gate opened |

A request during a run restarts at stage 1 with the gate closed (every stage is idempotent); several
requests before a poll are one run. An owner that is not running (`Device::va_up` / `display_up`,
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
`gop=off` (no boot layer, no seed) nothing changes at unload.

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
| BAR0 shadow pieces, hole pages | B | zero + `fill_piece` (the realize fill) + republish | vCPU, drainer, VA, display | gate; publication is drainer-owned | I (fill shared by construction) |
| token words, rung bitmap, kernel-token set (plane) | B | retired by the frees; swept fresh at Finish (`reset_tokens`) | vCPU ring, workers claim, act retire | gate; BUSY words waited out by the free path | M |
| `VmCaps` (channel budget) | B counts | released by the frees, **not zeroed**: a count left is a host channel still alive (leak, counted) | act | — | I |
| Passthrough twins, Translated slots, relays, USER rings, host TSGs, debuggers, CUDA limit, encoder sessions (`ChanPlane`) | B (host objects) | freed through `ChanPlane::free(c, c)` per guest client on the act thread; `LiveCensus` logged | act, drainer (statements), workers (pump) | frees run behind queued acts (a birth in flight lands first, then is freed); the free path stops pumps and waits out BUSY tokens | I (needs a host) |
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
| guest VRAM contents other than `[0, G)` and our roots | (guest data) | **kept** (owner decision D1, §9) | — | — | I |

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
   preempts and unbinds the channel (stage 3). No completion is written for it. ⚠ **Residual
   window**: QEMU resumes the guest as soon as the hold phase returns; until stage 3 frees the twins
   (inferred: milliseconds), a host channel can still read the GPFIFO and write semaphores in guest
   RAM that the next life's firmware is starting to use. Real hardware stops DMA at reset. Closing
   it needs either a blocking host call under the BQL (forbidden) or holding the vCPUs; owner
   decision D2. [inferred]
4. **Guest BAR0 writes during the reset** are dropped; BAR0 reads come from the shadow and show the
   old life's values until Finish (reads never exit). Firmware does not touch kf3's BAR0 (kf-gop draws
   only through BAR1); the guest driver loads seconds later. [inferred]
5. **BAR1 `[0, G)` before stage 4** shows the old life's placement (or scratch): firmware drawing in
   that window goes there and is lost; the seed is placed again at stage 4. [inferred]
6. **A guest that resets in a loop**: each run is bounded; lines are bounded (`reset::loud`: the first
   16, then powers of two). [measured: the bound]
7. **Display copy in flight**: the worker resets after its completion (polled). [inferred]
8. **Start-up**: the reset runs once at machine creation (the hold phase); an owner not yet running
   is not waited for. If the reset path itself is broken, EVERY boot shows it (the gate stays closed
   or a stage line is missing) — a strong falsifier for §8. [inferred]

## 7. GPU-free tests (all passing at this branch's head)

Command (disk-saving flags used here because `/data` was full):
`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p kf-qemu -p kf-gsp -p kf-core -p kf-trap -p kf-mem -p kf-disp -p kf-rm`.

- `kf-qemu` `reset::tests`: `an_open_gate_admits_and_counts_the_entry_until_it_leaves`,
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
Expect `device reset #1: … GSP Cold -> Cold`, `act device reset: barrier: device reset epoch 1:
channel plane EMPTY`, `device reset #1: memory plane — … 0 VA space(s) retired, 0 KEPT, mirrors left 0`,
`device reset #1 COMPLETE`, then a normal boot. *Falsifier:* any stage line missing, a `WAITING at`
line, or the guest's GSP boot failing where `master` boots.

**L1 — Linux, QMP `system_reset` while Running, ×5.** Fast guest (or the display lane VM with
`gop=on,display=on`), run a CUDA workload (`cuda_ladder.sh` rung 1) in a loop, then
`{"execute":"system_reset"}` on the QMP socket; wait for the guest to boot; repeat 5×.
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

**L3 — Linux driver unload/reload.** `rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia`, wait,
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
while disabled; `NOT EMPTY`/`KEPT`; a TDR or 4101 in the first app run after a reset.

**L6 — races (stress, Linux).** QMP `system_reset` issued (i) during the GSP boot (within 2 s of the
driver load), (ii) during a CUDA run with doorbell-ioeventfd on, (iii) twice within 10 ms, (iv) during
`rmmod`. *Falsifier:* a `WAITING at` line, a hung drainer heartbeat, a second life that fails.

## 9. Decisions that belong to the owner

- **D1 — guest VRAM across a reset.** The reset zeroes only our roots and the boot framebuffer
  `[0, G)` (as realize did); the rest of the store keeps the previous life's contents (same VM, same
  tenant; real GPUs keep VRAM across a reset). Scrub the whole store at reset (a GPU memset of fb-mb,
  inferred tens of ms) or not?
- **D2 — the DMA window (failure mode 3).** Accept the milliseconds in which an old host channel can
  still touch guest RAM after QEMU resumes the guest, or stop the vCPUs until stage 3 finishes (a
  `vm_stop`-style hold — not a blocking call under the BQL, but it delays the guest's restart)?
- **D3 — unload with `gop=off`.** Unchanged (Preserved/black as B5 measured). Should a driver unload
  without a boot display also end the display life (black instead of the preserved console)?
- **D4 — tier B at an unclean driver unload.** `THE_CONSTRAINTS.md` §40 item 2 asks tier B to reset on
  driver unload "clean or unclean". This branch resets tier B at a device reset, and the display at
  fn 47; it does NOT free leftover channel twins at fn 47 (a clean unload has freed them; an unclean
  one sends no fn 47). Free leftovers at fn 47 too?

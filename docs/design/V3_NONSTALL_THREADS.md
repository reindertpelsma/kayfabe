# V3 — the non-stall rule: the register drainer and every input-serving thread never stall

**STATUS: BUILT, GPU-FREE-TESTED ONLY, 2026-10-09. Branch `claude/nonstall-sonnet-20261009` (off
`master` at `7040011a`, not merged). NOTHING HERE HAS RUN ON HARDWARE** — §7 lists the exact commands.
Built: E (measurement), A (slot lock), B (quiet logging), D (a bounded per-call doorbell snapshot), C (the act
thread as a strict-FIFO event loop). NOT built, owner's call: per-key act lanes (§5), a bounded logger thread (§3.B, only if measured needed).

**§9 (2026-10-09, branch `claude/drainer-verify-20261009`) verifies the register drainer from the code, not from
this document, and CORRECTS it here, above the text it corrects:** (1) §2 row I names `seal_shadow` as the GSP
lock's only other holder; `status_line` also takes it, with `try_lock`, for two short `format!`s. (2) §3.B's "production is
quiet" holds for an honest guest only: lines that a guest can repeat at ring rate are still plain `klog!` (§9 L2),
and a refusal-ledger overflow wrote one line per repeat (§9 L1b, fixed). (3) The `held_book` ledger is not "drainer
only" (§9 K2). (4) The object model is not bounded work: one `GSP_RM_FREE` can keep the drainer inside
`RmGraph::free_subtree` for hundreds of ms to seconds (§9 G1, OPEN). Open findings are listed in §9.4.

## 0. The rule (owner, 2026-10-09, binding)

The register drainer, and every thread that serves new input, may NEVER stall. A **stall** is any state
in which the thread cannot serve another input: a sleep, a timed wait, an acknowledgement wait, a
contended blocking lock, a blocking write, an unbounded loop. A wait inside an `epoll`/`select` that
also accepts new requests is **not** a stall. A 200 ms stall in the drainer is forbidden outright; the
same applies to the act thread (`kf3-chan-act`), because every guest RPC whose reply is a host act waits
behind it. Also binding (same day): no new filtering between host events and guest interrupts — events
go per subscription at NVIDIA's granularity, and an extra edge is fixed at its source.

`OWNER_RULINGS.md` §A.4 already said "no blocking on a vCPU and none under a lock another thread blocks
on"; this extends it to the drainer, the act thread, the workers and the doorbell servicer, and defines
the word.

## 1. Origin

An independent read-only audit of the Windows-branch tip (`origin/claude/windows-reset-20261009`,
`2e5ddc5c`) found five families of stalls (A–E below). The code is the same on `master`; the line numbers
differ and are given here for `master`. `NonstallArms`, `pt_stall_snapshot_poll` and
`deliver_preempt_complete` exist only on the Windows branch and are **not** ported (item F: report only).

## 2. Inventory — where a serving thread could stall (master `7040011a`)

| # | thread | site (master) | what blocks | status |
|---|---|---|---|---|
| A1 | drainer | `chan.rs` `ChanPlane::statement`, `Schedule` TSG filter | the per-channel slot `Mutex`, which a worker holds across a whole `pump` | **fixed** §3.A |
| A2 | drainer | `statement`, `Token` | same | **fixed** |
| A3 | drainer | `schedule_translated` (`lock()` for `scheduled`/`guest_idx`; `try_lock` for `stopped`, which read "not stopped" under contention) | same | **fixed** |
| A4 | drainer | free statement `try_lock … scheduled = false` (skipped under contention) | same | **fixed** |
| A5 | act | stop / restart / disable / evict / preempt / timeslice / bind / promote / deferred-ctx acts: `slot.lock()` | the same pump lock | **fixed** (they read `SlotMeta`; only `retire` still takes the lock, by `try_lock` + timer) |
| B | drainer, act, workers, VA, display | ~340 `eprintln!` (one per RPC on the drainer) | stderr lock; `write(2)` under throttling; ENOSPC/EPIPE panic kills the thread | **fixed** §3.B (quiet + never-panicking + measured) |
| C1 | act | `ChanPlane::retire`: 200 × `sleep(1 ms)` with the slot lock held | a sleep | **fixed** §3.C (timer continuation) |
| C2 | act | `DbFast::deregister`: `recv_timeout(2 s)` | an acknowledgement wait | **fixed** (`begin_deregister` + fd wait) |
| C3 | act, main loop | `DbFast::register`/`begin_deregister`, `site_add`/`site_del`: `KVM_IOEVENTFD` under the registry lock | an SRCU grace period in the kernel | **open** §5 (a blocking syscall; measured as `act_run`) |
| C4 | act | every RM verb an act makes | host RM's API lock / a GSP RPC in `nvidia.ko` | **open** §5 |
| C5 | drainer | held replies: `release_held` stops at the first reply whose act is pending | the FIFO of held replies | **open** §5 (head-of-line, not a stall of the drainer) |
| C6 | act | brief `lock()` on `pt`, `pt_objs`, `by_obj`, `scopes`, `caps`, `groups` (the drainer holds them for microseconds) | a contended std mutex | **open, measured**: `act_lock_wait_max_us`, `lock_wait_worst` |
| D | drainer | `DbFast::service_ready` repeated while 64 fds were ready | an unbounded loop | **fixed** §3.D |
| E | — | measurement | — | **built** §3.E |
| G | drainer | the GSP heartbeat: `rm.gpu_time_ns()` + two shadow stores every 500 ms | volatile reads of the mapped usermode page (NOT an RM ioctl — corrected here; an earlier commit message said ioctl) | moved to `kf3-status` (§3.B); not a stall source, moved for the drainer's sake |
| H | drainer | `status_line` every 2 s (`va_stats`, `chans.counts()` …) | locks | **fixed** (moved to `kf3-status`; `try_lock` only on the GSP lock) |
| I | drainer | `apply_register` → `self.gsp.lock()` and the GSP FSM's work; `deliver_rc`; `deliver_hotplug` | the drainer's own work (the only other holder is `seal_shadow` at init) | by design; **measured** (`apply_max_us`, `drain_pass_max_us`) |
| J | vCPU/main | `bar0trace` and `KF3_MAPLOG` print from a vCPU | stderr | default-off diagnostics, `PERTURBING_DIAGNOSTIC_ON` |

## 3.E Measurement (item E) — built

Always on, no flag, atomics only (`kf_chan::stall`). The status line (printed by the drainer every 2 s
when it changes) carries a `stall[...]` segment:

`drain_pass_max_us` (longest drainer iteration, loop top to park), `apply_max_us` (longest single
`apply_register`), `held_age_max_us` / `held_oldest_us` (longest hold of a guest reply, age of the
oldest held now), `act_wait_max_us` (queue wait before an act's first step), `act_run_max_us` (longest
synchronous act step), `act_total_max_us`, `lock_wait_max_us` + `lock_wait_worst=<name>:<us>` (longest
BLOCKED acquisition of any measured lock — `TimedMutex`/`TimedRwLock` — and which), the same for the
drainer and the act thread alone, `doorbell_batch_full`, and (from `kf-util`) `log[max_call_us …]`. Each gauge also
has an `over10ms` count (drainer pass, apply, act step).

Falsifiers (what a number would prove): `drain_pass_max_us` above a few ms outside the boot's first
seconds ⇒ the drainer stalled; `drainer_lock_wait_max_us` > 0 ⇒ the drainer blocked on a lock;
`act_run_max_us` ≥ 10 ms ⇒ one act step is a host verb that blocks (the lane proposal's evidence);
`log_dropped` > 0 ⇒ the logger was slower than the producers (a drop is what replaces a stall).

Tests: `kf_chan::stall::tests` (gauge, lock wait by name and role, rwlock, held-reply aging on an
injected clock, the fragment's names, scope timer).

## 3.A The slot lock (item A) — built

**Before.** A Translated channel's whole state sat behind one `Mutex<Slot>`. A worker holds it across
a whole `pump` (`StoreViews::view_for` → an RM `arm_cpu_view` ioctl + `mmap`, the USERD scan, the
completion probe). The drainer took it with a blocking `lock()` to read `tsg` (TSG schedule filter),
`guest_idx` (the `Token` statement) and to flip `scheduled` (`schedule_translated`), so a guest RPC
could wait for a pump. `serve` itself uses `try_lock` ("must never be contended"). The drainer's
`try_lock` sites had the opposite defect: a contended `try_lock` read `stopped == false` and skipped the
free's `scheduled = false` silently.

**After** (`crates/kf-qemu/src/slotcell.rs`). `SlotCell = SlotMeta + pump lock`. `SlotMeta` holds the
immutable `guest_idx`, `tsg`, host channel, guest engine and the birth-fixed host contexts, and the
atomics `scheduled`, `stopped`, `disabled`, `dead`, plus the act-only `ctx` mutex (never contended: only
the act thread writes it). The drainer touches only `SlotMeta` (`plan_schedule`, `stop_pump`,
`in_group`, `meta.guest_idx`); the status line and the BAR0-trace dump use `try_lock`. As a bonus the
act thread no longer takes the slot lock in any act except `retire` (stop, restart, disable, evict,
preempt, timeslice, bind, promote and the deferred context all use the meta).

**The disable guarantee, and how it is kept without blocking the drainer.** The guarantee: *after the
guest's `GPFIFO_SCHEDULE(false)` / STOP / `DISABLE_CHANNELS(true)` / EVICT has been answered, no further
work of that channel that was read BEFORE the disable is submitted* (a GP entry or pushbuffer segment the
guest then frees or reuses). The old blocking `lock()` gave it by making the drainer wait for the pump.
Now: the flags flip at once (atomics, `SeqCst`), and **the guest's reply is held** (a deferred act, like
every other host-act reply) until no pump that started before the disable is still running:
- a per-slot **pump epoch** (`SlotMeta::pump_epoch`): `serve` adds 1 right after it took the pump lock and
  BEFORE it reads the schedule flags, and 1 again when the pump ends (odd = running);
- the disable sets its flags, then reads the epoch (`quiesce_mark`): odd ⇒ a pump is in flight, the mark
  is that epoch. This is a Dekker pair on sequentially consistent operations — either the pump sees the
  flags (`may_pump()` false, it submits nothing) or the disable sees the pump (and waits for it);
- the wait is `slotcell::quiesce`, a **1 ms timer continuation** on the act thread's event loop (the
  drainer and the act thread keep accepting work; the held reply goes out when the epoch moves), with a
  deadline of `QUIESCE_TRIES` = 10 000 ms after which the disable is **refused by name** (never a silent
  success). With no pump running the answer is immediate (the drainer's reply stays `Done`).
- covered: Translated `GPFIFO_SCHEDULE(false)` (single and TSG-wide), `stop translated`,
  `DISABLE_CHANNELS(bDisable=TRUE)` over Translated rings, `evict translated context`. (A free is covered
  by `retire`, which waits for the BUSY token and the pump lock; the deferred-API context evict runs for a
  pump that has already returned `Pending`.) Passthrough twins have no CPU pump.
- under strict FIFO the wait also delays the acts queued behind it, by at most the length of one pump.

**Falsifier / tests** (`slotcell::tests`): `the_drainer_paths_return_while_a_worker_holds_the_pump_lock`
(a thread holds the pump lock; the token read, TSG filter, schedule flip and free-stop return within
2 s — each was a `lock()` before); its control `the_pump_state_is_locked_while_the_worker_holds_it` (a
blocking `lock()` in the same situation does not return, i.e. the old code path waits);
`a_stop_and_a_free_are_seen_under_contention` (the two silent `try_lock` defects); for the disable
guarantee: `a_disable_is_not_answered_until_the_pump_in_flight_has_ended` (a worker mid-pump, the
disable arrives, the reply is withheld while the act thread accepts other work, then released in order),
`a_pump_that_never_ends_gets_the_disable_refused_by_name_at_the_deadline`,
`a_pump_that_starts_after_the_disable_sees_it_and_submits_nothing`, and the 300-round race
`no_pump_forwards_after_the_disable_is_released`.

## 3.B Logging (item B) — built: production is quiet

**Rule (owner, 2026-10-09).** In steady state every input-serving thread (drainer, act thread, workers,
VA thread, display worker) logs NOTHING per RPC, per statement, per doorbell or per act. Allowed: a
bounded number of boot-phase lines (realize, first-N counters), a rare line that is itself the event,
rare error lines under "once"-style limits (a repeating error cannot flood), and anything behind an
explicit default-off diagnostic flag, which then makes the status line print
`PERTURBING_DIAGNOSTIC_ON(...)`. Non-blocking logging machinery (a logger thread, a queue, a ring, a
drop policy) is deliberately NOT built: QEMU itself logs synchronously (`qemu_log` = `flockfile` +
`fprintf`, a failed write ignored); quiet is the mechanism and the max-log-call counter is the witness.

**Before.** ~340 `eprintln!` in the VMM crates, one per RPC on the drainer (`GSP rpc`, `HELD-REPLY`, the
chanlink and inittables statement lines) and per statement/act on the act thread. `eprintln!` takes the
process-wide stderr lock, `write(2)` can sleep under dirty throttling, and on `ENOSPC`/`EPIPE` it
PANICS, killing the calling thread (for the drainer, silently: there is no `panic=abort` and no
`catch_unwind` at the FFI thread entry).

**After** (`kf-util/src/log.rs`).
- `klog!` — one `write(2)` to stderr, a failed write ignored (never a panic), the call's duration
  recorded in always-on atomics per thread class (`set_class`: drainer, act, worker, va, display).
  Status line: `log[max_call_us= max_call_class= drainer_calls= drainer_max_us= drainer_over1ms=
  act_max_us= act_over1ms= worker_max_us= failed_writes=]`.
- `klog_limited!` — the first 4 calls of a call site, then each power of two: every repeating error
  line on a serving thread.
- `klog_trace!` — only with `KF3_LOG_VERBOSE=1`; every per-RPC / per-statement / per-act / per-object
  line. The bench lane scripts export `KF3_LOG_VERBOSE=1` by default (`${KF3_LOG_VERBOSE-1}`) because
  their gates and `rpc_diff.py` read these lines; production does not.
- Witness: `crates/kf-qemu/tests/no_raw_prints.rs` fails on any `eprintln!/println!/eprint!/print!/dbg!`
  in kf-qemu, kf-chan, kf-rm, kf-gsp, kf-mem, kf-host, kf-core, kf-trap, kf-linux-raw (allowlist:
  `ioctltrace.rs`, the `KF_IOCTL_TRACE` diagnostic written from an abort path; `kf-cuda`'s two realize-time
  lines and the binaries are out of scope). `kf_util::log` tests: a writer that fails with `ENOSPC` is
  counted and ignored; a slow write is the recorded max for its thread class; `limited` and `trace`.

**Audit of every call (338 sites).**

| kind | count | decision |
|---|---|---|
| realize / boot-phase / once-per-device (`kf3: guest GPU UUID`, host-facts, display plane up, `w#n` for the first 512 privileged writes, GSP phase changes, `at stop`) | ~170 | kept (`klog!`): bounded by construction |
| lines already behind a default-off flag (`KF3_MAPLOG`, `KF3_BAR0_TRACE`, `KF3_PROF`, `KF3_COMPLETION_PROBE`, `KF_VAS_CENSUS`, `rpc_trace`, `KF3_TSHADOW`, display `trace`) | ~45 | kept; the flag is listed in `PERTURBING_DIAGNOSTIC_ON` |
| repeating error / refusal lines (act REFUSED, birth REFUSED, DEAD, hotplug/RC refused, W349REFUSE, HOST-ABI REFUSED, display REFUSED, UNSERVICED …) | ~70 | `klog_limited!` |
| per-RPC / statement / act / object lines (`GSP rpc <fn> seq`, `act …: …` incl. the BORN line, `GPFIFO_SCHEDULE`, BIND, PROMOTE, chanlink, hoststub, GSS census, mem `walk`/`apply`/`mirror`, `bar1db view`, `GATE ticket`, DEFERRED-API, display `statement`/`heads`/fps) | ~55 | `klog_trace!` (default off) |
| bounded already (HELD-REPLY / -POSTED: now first 16 + powers of two; NSI RELAY; fn-47; `SHOWN_LINES`; map_scattered first 32) | ~8 | kept |
| lifecycle lines the gates read: `DOORBELL-LEDGER` (one per channel free) and `RETIRED` | 3 | **kept unconditionally — a judgement call**: rate = channel lifecycle, not traffic, and `run_fast_guest.sh` gates on the ledger. Gate them behind `klog_trace!` if the owner wants zero |

**The periodic status line and the GSP heartbeat left the drainer.** A `kf3-status` thread
(`Device::housekeeping_loop`, not an input-serving thread) prints the status line (every 2 s, on change)
and stores the GSP heartbeat mailboxes every 0.5 s. The status line only reads atomics and `try_lock`s;
a busy GSP lock shows the last phase seen instead of flickering "busy". ⚠ A stalled drainer no longer
freezes the heartbeat the guest reads (it used to, by accident).

*Is the heartbeat safe off the drainer? (checked in code)*
- `gpu_time_ns` is **not an RM call**: three `load_u32`s of the host's mapped, read-only usermode page
  (`kf-host/src/lib.rs`), re-read until TIME_1 is stable. No lock, no ioctl, `&self` on an immutable field;
  safe on any thread. (An earlier commit message and the first version of this section called it an RM
  ioctl; that was wrong.)
- `shadow_store` is `piece_for` (a binary search of `OnceLock<Vec<Piece>>`, sealed before any thread
  starts — lock-free) plus `RawRegion::store`. The vCPU trap path calls the same function concurrently, so
  it was already multi-threaded. `store` of 4 bytes is now ONE aligned `write_volatile` of a `u32`
  (`raw_unsafe.rs`; it was a `copy_nonoverlapping` of 4 bytes, a single `mov` in practice but not
  guaranteed) — no tearing.
- **Race with the drainer's `publish`:** yes, on the same words. `GSP_MAILBOX0/1` (`0x110804/8`) are the
  `GspFalconMailbox0/1` registers, in `GspReg::FIXED`, so `publish` stores the FSM's answer there when it
  changed (or right after the guest wrote them: `apply_register` forgets the published value). The two
  writers are independent single-word stores; the last one wins. That is the outcome set that existed
  before the move (publish after a heartbeat on the next apply; a heartbeat after a publish ≤ 0.5 s
  later), now at finer interleaving. It is harmless because the FSM never reads the shadow — it takes the
  guest's mailbox writes (the libos boot args) from the privileged ring, not from the shadow word — and
  the guest does not read those words back until it uses them as heartbeats (595.84+), where either value
  is a fresh or at worst 0.5 s-old clock.
- **Ordering relative to the guest's read:** none is promised or needed. The two stores are plain
  (volatile) stores in program order, mailbox0 then mailbox1; on x86 (TSO) a vCPU sees them in that
  order; the guest compares each word's progress separately. Nothing else is published behind them (the
  RPC queue's data-before-edge ordering in `publish` has its own release fence).

**Escalation (not built).** If a real run shows a log call over about 1 ms on an input-serving thread
(`log[... drainer_over1ms= act_over1ms=]` or `max_call_us`), add a bounded logger thread: `klog!`
formats and `try_send`s into a bounded queue, dropping the newest line and counting it; a thread
does the writes. Until a run shows the need it is measured, not built.

## 3.D Doorbell servicing (item D) — built: a per-call snapshot

**Before.** `DbFast::service_ready` (called at the top of the drainer loop and before EVERY privileged
write is applied) repeated while a poll came back with a full batch (`MAX_READY_BATCH` = 64). A guest
ringing 64+ tokens continuously — each delivery re-arms its eventfd — kept the drainer there:
privileged writes, held-reply release, RC and hotplug delivery starved.

**After.** One call delivers every doorbell that is READY AT THE TIME OF THE CALL, **each registered
token at most once**, and returns. Implementation: polls (`epoll_wait(0)`, ≤ 64 fds each) with a
per-call set of tags already served; a token reported again (the guest re-rang after the snapshot) is
skipped and served by the NEXT call; the call ends on a short poll, on a poll that reports nothing new,
or after `registered + 1` polls. Work per call: at most one delivery per registered token (≤ the 256
placement budget), and polls bounded by the registry — never by how fast the guest rings.
`stall[doorbell_batch_full=]` counts calls whose first poll was full (more than a batch ready).

**The ordering invariant is kept** (`V3_DOORBELL_IOEVENTFD.md` §4): a doorbell a vCPU stored before a
later privileged write was signalled before the write was queued, hence before the sweep that precedes
applying it, so it is in the snapshot and delivered before the write — with any number of tokens ready.
(Reading a token's eventfd resets its whole count, so the first delivery covers every earlier store.)
Assumption pinned by a test: epoll's ready list is FIFO (a token ready before the call is reported
before one re-rung during it), so a poll with nothing new means nothing fresh is left.
*Correction of the first version of this item:* it delivered ONE batch and returned, which weakened this
guarantee for more than 64 ready tokens; that was wrong and is replaced by the snapshot.

**Tests** (`dbfast::tests`):
`a_doorbell_signalled_before_a_privileged_write_is_delivered_before_it_even_with_over_a_batch_ready` (a
real `Plane::drainer_pass`; all 100 doorbells precede the write — the one-batch version delivered 64, the
test failed before the fix); `a_call_delivers_every_ready_token_once_and_returns_while_the_guest_keeps_ringing`
(exactly 100, the re-rings are the next call's); `a_guest_ringing_as_fast_as_it_can_cannot_keep_one_call_running`
(a second thread signals all 100 fds in a tight loop: deliveries ≤ 100, polls ≤ 101, the call returns);
`control_the_old_loop_never_returns_while_the_guest_keeps_ringing` (the old repeat-while-full loop does not
end until a 300 ms cut-off).

## 3.C The act thread as an event loop (item C) — built, strict FIFO

**Before.** `kf3-chan-act` ran each act to completion. Two acts waited inside their step: `retire`
(200 × `thread::sleep(1 ms)` for a token a worker still held BUSY, with the slot lock held) and
`DbFast::deregister` (`recv_timeout(2 s)` for the drainer's acknowledgement). During either, the thread
read nothing — every act queued behind it, and so every guest RPC whose reply is a host act.

**After** (`kf-chan/src/actloop.rs`). An act is a first continuation plus a `finish` callback. A step
returns `Done(result)` or `Wait(Timer | Fd+deadline, next)`. The loop `epoll`s on its submission eventfd
and on the waited-for fd (the drainer signals an eventfd after it acknowledges a removal), so **a new act
is accepted the moment it is submitted, even while one waits**, and nothing sleeps. `retire` and `free`
are state machines (`retire_steps` → `await_removal` → `retire_busy` (1 ms timer, ≤ 200) → `retire_lock`
(`try_lock`, 1 ms timer) → the synchronous host frees; `free_stage` for the Passthrough twins). The
status line carries `actq[accepted finished waits depth=now/peak wait_max_us]`.

**Ordering: strict FIFO, deliberately.** One act is current at a time; the next starts when it finished,
so acts take effect in STATEMENT ORDER for every dependency key (client / object / channel) — the
documented invariant — with no cross-key reordering to prove. This removes the *stall* (the thread is
never unresponsive and a wait costs no thread) but not the *head-of-line wait*: an act queued behind a
waiting one starts after it. A wait is short in practice (the drainer's ack: microseconds to a drainer
pass; a BUSY token: the length of one worker pass) and is now visible: `actq[wait_max_us]`.

**Tests** (`actloop::tests`, `dbfast::tests`):
- `a_second_act_is_accepted_within_milliseconds_while_the_first_waits_on_a_timer` — the first act waits
  400 ms, the second is accepted within 100 ms (`accepted==2`, `depth==2`), nothing finished out of
  order, completion order = submission order.
- `control_a_step_that_sleeps_accepts_nothing_meanwhile` — the old shape (a step that sleeps 300 ms): the
  second act is NOT read for the whole sleep, and `act_run_max_us` shows it. This is the behaviour the
  retire/deregister waits had.
- `an_ack_wait_resumes_on_the_signal_and_at_the_deadline_when_none_comes`, `retry_waits_on_timers_and_gives_up_after_its_tries`,
  `stop_ends_the_loop_while_an_act_waits`.
- `begin_deregister_returns_at_once_and_the_ack_arrives_on_its_eventfd` — with no drainer running the
  old `deregister` blocked 2 s; the new call returns at once, the ack arrives on the fd, carries the final
  drain's ledger, and a lost ack is reported at the deadline.

**Not testable GPU-free:** the `retire`/`free` state machines themselves (they need a host RM); their
building blocks are tested above. Hardware: §7.

## 5. Open: which acts block inside host RM, and a proposal for per-key lanes (OWNER DECIDES; not built)

**Acts whose synchronous step can block inside host RM (an RM ioctl into `nvidia.ko`, which serialises
on host RM's API lock and may issue a GSP RPC) — all are `act_run`:**

| act (label) | blocking verbs | worst case |
|---|---|---|
| `birth passthrough`, `birth translated` | TSG/channel alloc, ring object alloc + map + CPU view, GR/video context creation (golden-context init), `schedule_enable` | the longest: a first GR channel creates the host GR context (100s of ms) |
| `engine object` | `rm.alloc` of the engine class (GR3D/compute/copy/video) | GR objects: context promote |
| `free` | `rm.free`, `free_channel`, `release_host` (unmap + object free + view release), `perf_cuda_limit`, encoder sessions; the fast-path deassign (`KVM_IOEVENTFD`, an SRCU grace period) | a free waits for the channel's work to drain |
| `preempt`, `ctxsw preemption` | `NVA06C PREEMPT(bWait=1)` | **unbounded if the GPU context is hung** |
| `stop`, `stop translated`, `disable channels`, `schedule`, `restart translated`, `evict ctx`, `evict translated context` | `DISABLE_CHANNELS`, `GPFIFO_SCHEDULE` (RM RPC to GSP, runlist update) | ms |
| `timeslice`, `channel timeslice` | `SET_TIMESLICE` | ms |
| `debugger`, `debugger exception mask`, `zcull bind`, `cuda limit` | alloc / control | ms |
| `display-SW twin`, `bind/promote translated context` | alloc / none | ms / none |

**What a blocked ioctl does to every queued act.** Serial head-of-line blocking: the thread is inside
`ioctl(2)`, so the single FIFO is stopped; each act behind it — from every client and channel — waits, and
so does the guest RPC (its reply is held, `release_held` is FIFO, so every later reply is held behind the
first pending one too). One hung `PREEMPT` stops every other guest process's allocations and frees.
`stall[act_run_max_us, act_wait_max_us]` measure it; the strict-FIFO event loop cannot remove it.

**Proposal: N lanes keyed by an explicit dependency key, FIFO within a lane.**
- *Key.* Every act declares `(client, resource)` where `resource` is its channel's host token (a
  channel's own acts: schedule/stop/disable/evict/preempt/timeslice/free/promote), or the client alone
  for client-scoped acts (birth/engine object/debugger/limit/encoder). Lane = `hash(client) % N`, with
  `N = 4`; a cross-client act (the ones that name two clients: none identified; `dup` is served by the
  graph, not an act) takes ALL lanes (a barrier).
- *Order argument.* Within one client every act lands on one lane, in statement order — the invariant
  holds per client (and so per object and channel, which belong to one client). Two clients' acts may
  interleave: that needs independence, which holds on host RM objects (a client's handles are its own),
  and must be CHECKED for the shared state: (1) the guest chid heap — a token index freed by client A and
  reborn by client B — the free's reply is held until its act resolves, so a serial guest RM cannot reuse
  the chid before; a hostile guest can, so `birth` must first take the token in the `caps`/plane table
  (it does: `allocate_channel` fails on a live token) and a free must remove it before the act (the
  Passthrough free does, at the statement); (2) the VA mirrors / T-space shared by clients of one guest
  VA space; (3) the host groups map keyed `(client, tsg, ctxshare)` — per client, safe; (4) `pt`, `scopes`,
  `caps` — maps behind their own locks, not ordered state.
- *What it buys.* A hung preempt of client A stops only A's lane; with 4 lanes, a hash collision still
  stops 1/4 of the clients. It does NOT help a single client (CUDA process) whose own acts queue.
- *What it costs / risks.* A second and later lane run host-RM calls concurrently; host RM serialises on its
  API lock anyway (measured 1.00x scaling for workers, `THE_CONSTRAINTS.md`), so the gain is liveness
  isolation, not throughput. The ordering argument above rests on (1)–(4) and on there being no act that
  names two clients; both need an audit of the 24 act sites I could not make with confidence without a
  hardware run (the ladder's multi-process lanes are the test) — hence "proposal".
- *Alternative.* Keep one lane; give `preempt` a deadline (`NVA06C PREEMPT(bWait=0)` + a poll by the
  event loop), which removes the one unbounded verb; every other act is bounded by host RM.

## 6. What remains open

1. Per-key lanes (§5) — owner's decision. 2. (closed: the doorbell order is kept by the snapshot, §3.D.)
3. `KVM_IOEVENTFD` under the registry lock on the act thread and the main loop (C3) — a blocking syscall
by nature; a helper thread for placements would remove it from the act thread. 4. The act-thread
`lock()`s on the plane maps (C6) — measured, not removed. 5. `DOORBELL-LEDGER`/`RETIRED` lines are kept
unconditionally (§3.B). 6. A bounded logger thread, if `log[... over1ms]` shows the need. 7. The Windows
branch's `NonstallArms`, `pt_stall_snapshot_poll`, `deliver_preempt_complete` are NOT ported (item F:
`NonstallArms` ordering is a separate branch).

## 7. Hardware verification (later, one box, strictly serial — NOT run by this branch)

Every GPU lane takes the shared GPU lock itself (`exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9`
inside `build_kf3.sh`, `run_fast_guest.sh`, `broker_lane.sh`, `interactive.sh`): run the steps one after
another, NEVER holding that lock in the calling shell (a held parent lock deadlocks the child's `flock`).
Rent per `scripts/bench/box/README.md`. Order:

```
# 1. GPU-free, anywhere
cargo test -p kf-util -p kf-chan -p kf-core -p kf-rm -p kf-gsp -p kf-mem -p kf-host -p kf-qemu
bash scripts/ci/clippy.sh && python3 scripts/ci/dependencies.py
# 2. on the box
bash scripts/bench/v3_gates.sh                                  # 9/9
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3   # KF3_RC=0
KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh nonstall 180      # 30/30 (rebuild the raw client and fast guest first)
# 3. the Linux guest with the display broker (owner: retest occasionally so it keeps working), after 2.
bash scripts/bench/display/broker_lane.sh prep <nvkvm-pv-rev>   # once per box
bash scripts/bench/display/broker_lane.sh run nonstall_brk      # evidence: /workspace/bench/brk/nonstall_brk/
bash scripts/bench/display/interactive.sh prep                  # once per box
bash scripts/bench/display/input_proof.sh nonstall_input        # evidence: $WORK/proof-nonstall_input/proof.log
bash scripts/bench/display/interactive.sh                       # the owner's window; Ctrl-C / `interactive.sh stop`
#    with that window up, vkcube in the guest (the cube must be animating IN the broker window):
ssh -i /workspace/bench/guest_key ubuntu@192.168.77.2 'sudo -u ubuntu env DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority timeout 30 vkcube --c 300; echo RC=$?'
#    and the scripted flip/pace check (X11 vkcube FIFO + IMMEDIATE, glxgears), headless display lane:
DISPLAY_HOOK=max_fps_x11_hook DISPLAY_KF3_EXTRA=x11-dispsw=on bash scripts/bench/display/lane.sh nonstall_x11
# 4. fat-guest lanes the owner already runs: cuda_ladder.sh, the app matrix, llm_parity.sh, gfx_suite.sh, video_lane.sh
# 5. a Windows boot (scripts/bench/windows/win_vm.sh) and read the stall[...] segment of the status line
```

**Pass criteria.**
- `v3_gates.sh` 9/9; `build_kf3.sh` `KF3_RC=0` with the revision installed under `kf3-bins/<rev>/`;
  `fast_suite` 30/30; each lane's claim cites that revision.
- *Broker lane (`broker_lane.sh run`, `V3_DISPLAY.md` §8.9/§8.17: what "working" means):* `BRK_GUEST_DESKTOP=`
  reports the guest desktop up (Cinnamon) and `BRK_WINDOW id=[…]` is a window (not `none`); `BRK_RUNGS`
  shows frames delivered; `BRK_RELAY_CONNECTED` ≥ 1; the host-vs-guest frame diff lines
  (`BRK_HOST_VS_GUEST`) show the desktop on the host; vkcube flips: the ssh command above prints `RC=0` and
  the cube animates in the broker window, and `lane.sh nonstall_x11` prints `D4_*_B3_VKCUBE_FIFO_480 RC=0`
  and `D4_*_B5_VKCUBE_IMMEDIATE… RC=0` (no `Assertion`) with a non-zero `fps=[…]`; no `did not complete` / `scanout REFUSED`
  in `qemu.log`; 0 Xid in the host dmesg.
- *Input (`input_proof.sh`):* `PROOF_GRUB_KEY PASS`, `PROOF_KEYS PASS`, every `PROOF_ABS` within 2 px of
  the scaled position, `PROOF_REL_X11 PASS`, `PROOF_ABS_UNDER_GRAB PASS_DROPPED`, `PROOF_DESKTOP`
  shows cinnamon up; `PROOF_EXIT`/`EXIT` present with `qemu_left=0`.
- *Non-stall witnesses, from the status line of any of the above (printed by `kf3-status`):*
  `stall[drain_pass_max_us=… apply_max_us=… held_age_max_us=… act_wait_max_us=… act_run_max_us=…
  lock_wait_max_us=… drainer_lock_wait_max_us=0 …] log[max_call_us=… drainer_over1ms=0 …]` and
  `actq[… wait_max_us=…]`. Falsifiers: `drainer_lock_wait_max_us > 0` ⇒ the drainer still blocks on a
  lock (see `lock_wait_worst=`); `drain_pass_max_us` over ~10 ms outside the first seconds of boot ⇒ a
  drainer stall (read `apply_max_us` and `lock_wait_worst`); `act_run_max_us` ≥ 10 ms ⇒ an act step is a
  blocking host verb (§5); `log[drainer_over1ms>0]` ⇒ add the bounded logger thread (§3.B);
  `doorbell_batch_full` rising with `drain_pass_max_us` ⇒ the guest rings more than 64 tokens at once.
- A Windows boot: the same counters, with `KF3_LOG_VERBOSE=1` only if the RPC trace is wanted
  (`win_vm.sh` exports it by default).

## 9. Verification of the drainer (2026-10-09) — who can stall `kf3-drainer` for a second

**STATUS: LIVE, 2026-10-09. GPU-free: read from the code of this branch (`e864ab26`; the tests and one
fix are the two commits above this one) and tested where a test can be built. Nothing ran on hardware.** Tags: **[T]** measured by a test,
**[R]** read from the code (file:line), **[I]** inferred. The question was asked after an earlier read-only audit
(of `origin/claude/windows-reset-20261009`); none of its claims is taken on trust below.

### 9.1 How the call graph was enumerated, and how I know I did not miss a path

Entry: `Device::drainer_loop` (device.rs:2252): `DbFast::service_ready` → `Plane::drainer_pass(256)` →
`HostOps::apply_register` ×≤256 (device.rs:3268) → `release_settled` → `deliver_rc` → `deliver_hotplug` → park
(`try_park`, `Poller::wait(50 ms)`, `on_ready`). Method:

1. **Crate boundary.** `kf-gsp`, `kf-rm`, `kf-abi`, `kf-arch`, `kf-chip`, `kf-disp`, `kf-trace`, `kf-core`, `kf-trap`
   depend on neither `libc`, `kf-host`, `kf-linux-raw` nor `kf-cuda` (`Cargo.toml`), so a call in them reaches the
   OS only through `std`. A scan of their non-test source (comments and `#[cfg(test)]` removed) for `Mutex`, `RwLock`,
   `Condvar`, `lock(`, `recv`, `wait(`, `join(`, `sleep`, `park`, `std::fs/net/process/thread/io`, `mpsc`, `libc::`
   found: 7 mutexes (K-table), `/dev/urandom` (`gpuuid.rs:311`, realize only), `std::env::var` reads
   (`inittables.rs:1907`, `barpde.rs:482`, `sw_runlist_host.rs:41`: the env read lock, no writer at run time) and
   `OnceLock` first-use tables (`kf-abi/versions.rs:573`, `kf-disp`, `kf-chip/hwref.rs`: a one-time compute). **[R]**
2. **`dyn` boundary, shipped configuration** (`device.rs:871-938`): `CommandPolicy` = `ReselectAtFn1` →
   `ControlCensus` → `StickyAnswerGuard` → `PolicyChain[HostStub, FecsTrace, (SwRunlistHostOwned, GfxPoolProbe,
   sw-runlist probe: env, off), (Display if display=on), Channel, PageDir, Sysmembar, BarPde, Zbc, VfGuest,
   Observing(FaultBuffer), Observing(OsEvent), InitTable, StaticInfo, GuestSystemInfo, Inert, Object(GraphObjects),
   Unserviced]` (`kf-rm/lib.rs:458-604`). The injected callbacks: `ChanSink` = `ChanPlane::statement`
   (device.rs:919), `MemSink` = `Inbox::push` (device.rs:911), `GuestRam` = `Ram(Device)`, `GuestRamAuthority` =
   `MemoryListRam` (probe, env, off), `Sink` = `Device::deliver` (device.rs:3210), `HostOps` = `Device`,
   `Ioeventfd` = `IoeventfdHook` (called by the act thread and the main loop only: dbfast.rs:515,587,681,771,792),
   `OrphanUndo` = `attach_dispsw_withdraw` (x11-dispsw, off), `GspModel`/`BootSequence` = pure `kf-chip` rows.
   `kf-rm` never receives a `HostControls` on this path: `rmfacts::host_facts` runs once at realize (device.rs:357).
3. **Function by function** for `kf-qemu` and `kf-chan`: for every drainer-reachable function I read its body and every
   lock / syscall / channel / eventfd / loop in it, then swept `kf-qemu` (including `.lock()` chains split across
   lines) and checked the holder of each lock (K-table). Guard-scope scan: no lock guard bound in `chan.rs` /
   `device.rs` is live across a `.rm.` call (the only candidates were false positives of a heuristic; the
   `rows.write().map(|mut r| …)` in `engine_object` consumes its guard before `rm.unmap`). **[R]**

### 9.2 The table

| # | item | where | stall ≥ 1 s? | evidence | test |
|---|---|---|---|---|---|
| D1 | `drainer_pass` loop | kf-core plane.rs:553, device.rs:2309 | no, bounded by 256 writes | budget, not queue length or producer rate **[T]** | `kf-core` `a_drainer_pass_applies_at_most_its_budget…` |
| D2 | doorbell servicing | dbfast.rs:925-1038 | no: `side` is the drainer's alone; `epoll_wait(0)` ≤ registered+1 polls of ≤64; eventfd read/write are `EFD_NONBLOCK` (host_fd_unsafe.rs:485); `deliver` = atomics + one `HostRm::doorbell` store (kf-host lib.rs:862) or a non-blocking eventfd write. The registry lock `reg` and `tx` are held across `KVM_IOEVENTFD` by the main loop / act thread and are **never** taken by a drainer function **[T]**. `sync()`'s `while try_recv` (dbfast.rs:851) ends when the act thread's queue is empty: bounded by the act rate, not by code **[I]** | `the_drainer_serves_doorbells_while_another_thread_sits_in_kvm_ioeventfd` (1.3 s in KVM, all calls < 50 ms, control: a registry reader waits), `dbfast_drainer_lock_takers` ×2 |
| D3 | hand-off to the act thread | actloop.rs:131 | no: unbounded `mpsc::send` + non-blocking eventfd write | **[T]** | `submit_returns_at_once_while_the_act_thread_is_stuck_in_a_blocking_step` |
| D4 | `apply_register` | device.rs:3268 | no wait of its own; **one doorbell may carry up to 4096 RPCs** (`avail ≤ msgCount`, region ≤ `max_entries` 4096 pages, element ≥ 4 KiB: boot.rs:1056,1910, ring.rs:256-262) | each RPC's cost is G/L below **[R]** | — |
| D5 | GSP FSM | kf-gsp boot.rs | no | loops bounded: `RegionMap::load` ≤ 4096 (ram.rs:126), `large` ≤ 64 continuations, `publish` ≤ `max_entries`; refusal ledger ≤ 128 rows **[R]** | — |
| D6 | guest RAM `Ram(self)` | device.rs:3174 → mem.rs:136 | no software wait; a memcpy ≤ one element from QEMU's shared memfd mapping can page-fault on the host (swap, userfaultfd) **[I]** | `RamMap::blocks` read lock: K1 | — |
| D7 | policy chain syscalls | kf-rm | none (9.1 §1-2) **[R]** | — | — |
| D8 | `ChanPlane::statement` | chan.rs:2084 | no host call outside an act closure (26 functions scanned; 0 hits; control sees them in the act helpers) **[T]**; locks: K-table | — | `kf-qemu` `drainer_no_host_calls` ×3 |
| D9 | held replies | boot.rs:2526, device.rs:2824 | no: `outcome()` is an atomic; the orphan undo is the only caller-defined code under the lock and ships only for x11-dispsw (a queue send) **[R]** | — | — |
| D10 | `publish`, `shadow_store`, `deliver_*`, `latch_and_deliver` | device.rs:2179,1253,2924,3016,1709 | no: ≤ ~50 volatile stores, atomics, one eventfd write | **[R]** | — |
| D11 | park | wake.rs:73, device.rs:2341-2350 | not a stall by the rule (epoll that accepts doorbells and `drainer_efd`); `try_park` is a lock-free CAS loop | **[R]** | — |
| D12 | `HostRm::doorbell` on a vCPU (claim a) | device.rs:1456-1591, kf-host lib.rs:862 | no | see 9.3(a) | — |
| D13 | `bar0trace_dump` | chan.rs:4894 (device.rs `log_report`) | **yes, if `KF3_BAR0_TRACE=1`**: arms a CPU view (`openat` + `NV_ESC_RM_MAP_MEMORY` + `mmap`) and releases it (an RM ioctl) on the drainer, once per run. Default off; the status line then prints `PERTURBING_DIAGNOSTIC_ON` | **[T]** the scan names it as the one exception and that its only caller is behind `take_dump()` | `…the_one_drainer_side_host_call_is_the_default_off_bar0_trace_dump` |
| L1 | a log call lasts as long as its sink blocks | kf-util log.rs:178 (`Stderr` lock + `write(2)`) | **yes, if the sink blocks and the drainer logs** | **[T]** control | `control_a_log_call_lasts_exactly_as_long_as_its_sink_blocks` |
| L2 | guest-repeatable `klog!` on the drainer | `guestsysinfo.rs:174-191` (every fn 1, with 3×256 guest bytes), `lib.rs:321,332,404-416` (every fn 1), `kf-gsp/sysinfo.rs:94` (every fn 72), `staticinfo.rs:268-307` (every fn 65), `boot.rs:1714` (every boot-args publish), `device.rs:3364,3370` (every phase change / WPR2) | **yes, against a blocked sink**; the rate is the guest's | fn 1 ×100 → 100 drainer log calls **[T]**; the others **[R]** | `drainer_log_flood` ×2 (`#[ignore]`d findings) |
| L1b | refusal ledger overflow | kf-gsp refusal.rs:92 | was: one line per repeat past 128 rows | 1000 repeats → 1000 lines **[T]**; **FIXED** in this branch | `past_the_cap_a_repeated_refusal_is_not_fresh_again` |
| G1 | `RmGraph::free_subtree` fixpoint | rmgraph.rs:1885-1925 | **yes**: depth × table size; descending handles: 3000 deep = 333 ms, 10 000 deep = 4 s (release); the cap is 2^18 handles. Reachable from `GSP_RM_ALLOC` bytes (class, `hParent`, `hObject` are passed verbatim: rmrpc/mod.rs:1516-1525) | **[T]** | `one_rm_free_of_a_deep_chain_must_return_within_50_ms` (FAILS, `#[ignore]`d) |
| G2 | `resolve_pending_dups` + `pending_dups.retain` | rmgraph.rs:2004, 1950 | no: 11 ms per alloc at 262 000 parked dups, 1000× an honest alloc | **[T]** | `parked_dups_tax_every_alloc…` (passes) |
| G3 | `cache_targets` on every Device alloc | rmgraph.rs:1486, 1776 | no, but over 50 ms: 62 ms with 260 000 unrouted objects | **[T]** | `a_device_alloc_must_return_within_50_ms…` (FAILS, `#[ignore]`d) |
| G4 | per-free scans of guest-grown tables | chanlink.rs:1738-1744 (`vas_aliases.values().any`, unbounded insert at :1711), barpde.rs:333-359, rmgraph.rs:1950 | unknown | **[R]**, not measured | — |

**K-table: every lock the drainer can take, and every other taker** (a lock whose other holders hold it ≥ 1 ms across a
syscall / I/O / wait / unbounded loop is a finding; none of these is, except the stderr lock):

| lock | drainer sites | other takers → what they do while holding | worst case |
|---|---|---|---|
| `Device::gsp` (device.rs:245) | 3283, 2828, 2929, 3037 | `seal_shadow` 1225 (realize, before the guest runs); `status_line` 2430 `try_lock` (kf3-status and `kf3_status`): `format!("{:?}")` + `refusals().summary()` (≤128 rows) | µs **[R,T: scan]** |
| `held_book` (device.rs:294) | `note_held` 2816 ×3 per apply | `status_line` 2702 holds it across `Stall::fragment` (K2): ~25 formatted numbers | tens of µs; strict-rule violation by the letter **[R]** |
| `RamMap::blocks` (mem.rs:82) | every guest-RAM access (`block_for`) | `add`/`del` (QEMU main loop, topology change): a `Vec` insert/sort/retain; `MemoryListRam` (probe, off) holds a read guard over ≤ 4096 B | µs **[R]**; blocks if held: **[T]** K1 |
| `Inbox::q` (mem.rs:1867) | `push` (every page-dir statement) | VA thread `take()` = `mem::take` | µs **[R]**; K1 |
| `ChanPlane` `pt`, `pt_objs`, `by_obj`, `scopes`, `sw_objs`, `dbg`, `cuda_limit`, `enc_sessions`, `caps`, `mirrors`, `slots` (RwLock), `rc_queue` | the 26 functions of D8, `take_rc` every pass | act-thread closures: map insert/remove/clone of ≤ 64 channels (`VmCaps::from_declared(64,…)`, chan.rs:1774); worker `rc_scan` (chan.rs:4807) holds `pt` for ≤ 64 twins × 4 volatile loads (a VRAM view is an MMIO read: slow only if the GPU is wedged **[I]**); VA thread `mirrors` insert/remove, `pt_doorbells` `try_lock`; status `counts()`/`dispsw_status` | µs–sub-ms **[R]**; never across an `rm.` call (scan **[T]**) |
| slot pump lock (`SlotCell`) | none (atomics only) | workers, act thread (`try_lock` + 1 ms timer) | n/a **[R]**, existing `slotcell` tests |
| `Plane::kernel_tokens` (kf-core plane.rs:161) | `free_channel` 3841 (under `caps`) | worker `owner_of`, act `allocate_channel` | µs **[R]** |
| `DisplayModel` (kf-rm display.rs:116) | `DisplayPolicy`, `deliver_hotplug` 3022 | display worker: `take_statements`, `set_monitor` (EDID build) display.rs:2068,2649 | µs **[R]** |
| `defapi::Registry`, `UnservicedLog`, `ControlCensusLog`, `FaultBufferLog`, `OsEventLog`, `SharedRefusalCensus`, `SystemInfoCell`, `Deferred.undo` | the chain links | worker `lookup`/`executed` (registry); status thread `sample()` (unserviced); nobody else | µs **[R]** |
| `DbFast::side` | `service_ready`, `deliver_tags` | none **[T]** | — |
| process stderr lock + `write(2)` | `klog!` | every thread that logs (act, workers, VA, display, status) | the sink's stall (L1) |
| libc allocator, env read lock, first-use `OnceLock` | everywhere | everyone | unavoidable **[I]** |

### 9.3 Answers to the five claims

- **(a) True with a precision.** The vCPU Passthrough doorbell (device.rs:1456-1591): `bar0trace.note` (one atomic
  load when off), the timer-ignore decode, two relaxed counters, a range check, on Hopper+ the FSP-EMEM decode
  (`fspemem.rs:143`, atomics), `Plane::trap_write` (one token-word load → `Action::RingHostInline`,
  kf-trap trap.rs:127-134), then `HostRm::doorbell` (kf-host lib.rs:862: `release_fence` = compiler fence + `sfence`,
  one `store_u32`) and `note_inline` (1–2 relaxed `fetch_add`, chan.rs:1942). No lock, syscall, allocation or wait.
  With `KF3_MAPLOG` on, `note_inline` also `klog!`s from the vCPU (default-off diagnostic). **[R]** The same function runs
  on the drainer for fast-path doorbells (`Sink::deliver`).
- **(b) False.** The act thread is also fed by the **worker pump**: `deferred_ctx_act` (chan.rs:3462, called from
  `VaSplit::sw_start`, chan.rs:675) on a guest NV50_DEFERRED_API trigger in a pushbuffer, and by the drainer's
  `release_held` through the orphan undo (`attach_dispsw_withdraw`, chan.rs:1572; x11-dispsw only). Both are guest-caused,
  neither is a GSP statement. **[R]**
- **(c) True for production, with one named exception** (D13, `KF3_BAR0_TRACE`). The drainer's only host action is the
  doorbell store. **[T]** (scan) + **[R]**
- **(d) Yes, an RM ioctl.** `StoreViews::view_for` (chan.rs:394) on a miss calls `HostRm::arm_cpu_view` (kf-host
  lib.rs:1490) = `openat("/dev/nvidia<N>")` + **`NV_ESC_RM_MAP_MEMORY` (0x4E)** on the control fd
  (`arm_cpu_view_on`, :1552-1556) + `mmap`; when it already holds 8 views it first issues **`NV_ESC_RM_UNMAP_MEMORY`
  (0x4F)** (`release_cpu_view`, :1577). In nvidia.ko the map goes `Nv04MapMemoryWithSecInfo` → `rmapiMapToCpuWithSecInfoV2`
  → `serverMap` → `serverTopLock_Prologue` → `rmapiLockAcquire` on `g_RmApiLock` (a `portSyncRwLock`; write mode
  unless the module is allowed shared mode) — read in ogkm **595.84** (`rs_server.c:2005`, `alloc_free.c:222-252`,
  `rmapi.c:551-640`); the bench driver 580.159.04 was not re-read **[R, I for 580]**. So it can wait behind an act's
  RM verb (e.g. `PREEMPT(bWait=1)`), on a **worker**, under that channel's pump lock. The drainer takes neither (slot
  atomics, §3.A), and `Mem` is built only in `serve` (chan.rs:5297). The drainer's `bar0trace_dump` (D13) calls
  `userd_view` → the same `arm_cpu_view`.
- **(e) True, with `status_line`.** The GSP mutex is taken by the drainer (4 functions), `seal_shadow` (realize) and,
  with `try_lock`, `status_line`; no other file can name the field **[T]** (`the_gsp_mutex_is_taken_by…`).

### 9.4 The answer, and what is open

**Who can keep `kf3-drainer` busy or blocked for a second or more?**
1. **The guest, by sending the drainer an RPC whose object-model update is quadratic** (G1; G3 and the G4 sites are
   the same family, smaller). OPEN. Not tiny to fix (a parent→children index in `RmGraph`), so not fixed here.
2. **A blocked stderr sink, through any log line the guest can repeat** (L1 + L2). The quiet-production rule holds for an
   honest guest only. OPEN for L2 (`klog!` → `klog_limited!` is one token per site but changes what the Windows lane
   greps, e.g. the phase-change line prints a different phase each call, so I did not). L1b is fixed.
3. **The host kernel**, under a guest-RAM page fault or an allocator / env lock **[I]**; and the store behind `sfence`
   to a wedged GPU, which a vCPU suffers equally **[I]**.
4. **Diagnostics**, only if switched on: `KF3_BAR0_TRACE` (RM ioctl on the drainer), `KF3_PROF`, `KF3_MAPLOG`,
   `KF3_MEMORY_LIST_PROBE`.

**Nobody else**, from the code: not the act thread (every wait is a continuation; `submit` cannot block), not a worker
(the pump lock is not taken; its `NV_ESC_RM_MAP_MEMORY` stalls the worker, not the drainer), not host RM's API lock (the
drainer takes no RM ioctl), not KVM (`KVM_IOEVENTFD` runs under `reg`, which the drainer never takes), not `kf3-status`
(µs on `gsp` and `held_book`), not the VA thread or the display worker (µs on `Inbox::q`, `mirrors`, `DisplayModel`).

### 9.5 What I could NOT verify, and why

- **Whole-chain CPU cost.** Blocking primitives, syscalls and locks are enumerated exhaustively; the CPU cost of ~30 000
  lines of links and decoders (`chanlink` 2969, `inittables` 3283, `display` 1645, `barpde` 804, `rmgraph` 2415 …) is
  not. I found the quadratic sites by grepping per-event scans (`retain`, `values().any`, fixpoints), measured three, and
  list three more (G4). There may be others. The chain as a whole was not driven with hostile bytes.
- **`Device`-level behaviour** (`gsp`, `held_book`, `ChanPlane` locks, `apply_register`) needs `HostRm`, i.e. a GPU;
  the evidence there is the scans and the code.
- **Hardware stalls** (page faults, PCIe) and anything on a box: no stall[] counter was read.
- **The 580.159.04 nvidia.ko** was not read; the RM-API-lock claim is from 595.84 and the repo's own measurement
  (`THE_CONSTRAINTS.md:57`).
- A timing bound is a property of one build on one box (release, this CPU).

### 9.6 Tests added by this verification

Run: `cargo test -p kf-util -p kf-chan -p kf-core -p kf-gsp -p kf-qemu -p kf-rm` (1068 passed, 0 failed, 6 ignored).
Findings, which fail: `cargo test -p kf-rm --test drainer_log_flood -- --ignored --test-threads=1`;
`cargo test --release -p kf-rm --test drainer_object_model_cost -- --ignored --nocapture --test-threads=1`;
`cargo test -p kf-qemu --lib the_drainer_side_of_ram -- --ignored`.
Passing: `kf-chan` `the_drainer_serves_doorbells_while_another_thread_sits_in_kvm_ioeventfd`,
`submit_returns_at_once_while_the_act_thread_is_stuck_in_a_blocking_step`, `dbfast_drainer_lock_takers` (2);
`kf-core` `a_drainer_pass_applies_at_most_its_budget…`; `kf-util` `control_a_log_call_lasts_exactly_as_long_as_its_sink_blocks`;
`kf-gsp` `past_the_cap_a_repeated_refusal_is_not_fresh_again` (fails before the fix commit, passes after);
`kf-qemu` `drainer_no_host_calls` (3), `drainer_lock_takers` (2).

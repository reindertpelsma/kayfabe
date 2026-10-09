# V3 — the non-stall rule: the register drainer and every input-serving thread never stall

**STATUS: IN PROGRESS, 2026-10-09. Branch `claude/nonstall-sonnet-20261009` (off `master` at
`7040011a`). GPU-FREE-TESTED ONLY: nothing here has run on hardware.** Item E (the measurements) is
built; A–D are built in the commits that follow it; C (the act thread) is built as a strict-FIFO event
loop — the per-key lanes are a PROPOSAL for the owner (§5), not built. Each section says what is built.

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
| A1 | drainer | `chan.rs` `ChanPlane::statement`, `Schedule` TSG filter (`s.lock().is_ok_and(g.tsg …)`) | the per-channel slot `Mutex`, which a worker holds across a whole `pump` | built (§3.A) |
| A2 | drainer | `statement`, `Token` (`s.lock().map(g.guest_idx)`) | same | built |
| A3 | drainer | `schedule_translated` (`slot.lock()` for `scheduled`/`guest_idx`; `try_lock` for `stopped`, which silently read "not stopped" under contention) | same | built |
| A4 | drainer | free statement, `try_lock … g.scheduled = false` (skipped silently under contention: the pump kept running) | same | built |
| B  | drainer, act, workers | ~370 `eprintln!` in the VMM crates; one per RPC on the drainer (`device.rs` `log_report`, `apply_register`, `chanlink.rs`, `boot.rs`, `chan.rs`) | Rust's process-wide stderr lock; `write(2)` under dirty throttling; ENOSPC panics and kills the thread silently | built (§3.B) |
| C1 | act | `ChanPlane::retire` | 200 × `sleep(1 ms)` while the slot lock is held | built (§3.C) |
| C2 | act | `DbFast::deregister` | `recv_timeout(2 s)` for the drainer's acknowledgement | built |
| C3 | act | `DbFast::register`/`deregister`, `site_add`/`site_del` | `KVM_IOEVENTFD` (an SRCU grace period) under the registry lock | open, §5 |
| C4 | act | every RM verb an act makes (`alloc`, `map`, `free`, `schedule_enable`, `disable_channels`, preempt …) | host RM's API lock / a GSP RPC inside `nvidia.ko` | open, §5 |
| C5 | drainer | `deliver_rc`/`release_settled`: a held reply waits for its act | the FIFO of held replies (`release_held` stops at the first pending one) | open, §5 |
| D  | drainer | `DbFast::service_ready` | repeated while 64 fds were ready | built (§3.D) |
| E  | — | measurement | — | built (§4) |
| G  | drainer | `Device::drainer_loop` heartbeat: `rm.gpu_time_ns()` every 500 ms | a host RM ioctl on the drainer (mostly a register read; not proven non-blocking) | **open — found by this work, not in the audit** |
| H  | drainer | `status_line` (every 2 s): `va_stats`, `vat_prev`, `chans.counts()` | `counts()` takes every slot lock with a blocking `lock()` | built with A (§3.A) |

## 4. Measurement (item E) — built

Always on, no flag, atomics only (`kf_chan::stall`). The status line (printed by the drainer every 2 s
when it changes) carries a `stall[...]` segment:

`drain_pass_max_us` (longest drainer iteration, loop top to park), `apply_max_us` (longest single
`apply_register`), `held_age_max_us` / `held_oldest_us` (longest hold of a guest reply, age of the
oldest held now), `act_wait_max_us` (queue wait before an act's first step), `act_run_max_us` (longest
synchronous act step), `act_total_max_us`, `lock_wait_max_us` + `lock_wait_worst=<name>:<us>` (longest
BLOCKED acquisition of any measured lock — `TimedMutex`/`TimedRwLock` — and which), the same for the
drainer and the act thread alone, `doorbell_batch_full`, `log_queued`, `log_dropped`. Each gauge also
has an `over10ms` count (drainer pass, apply, act step).

Falsifiers (what a number would prove): `drain_pass_max_us` above a few ms outside the boot's first
seconds ⇒ the drainer stalled; `drainer_lock_wait_max_us` > 0 ⇒ the drainer blocked on a lock;
`act_run_max_us` ≥ 10 ms ⇒ one act step is a host verb that blocks (the lane proposal's evidence);
`log_dropped` > 0 ⇒ the logger was slower than the producers (a drop is what replaces a stall).

Tests: `kf_chan::stall::tests` (gauge, lock wait by name and role, rwlock, held-reply aging on an
injected clock, the fragment's names, scope timer).

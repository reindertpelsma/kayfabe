# Signal / interrupt delivery during forwarded ioctls — DESIGN (not yet implemented)

Status: **design captured, implementation deferred** (its own milestone). This is
*correctness*, not polish: without it a guest process that is Ctrl-C'd, times out,
or is SIGSTOP'd mid-GPU-call behaves differently from native Linux (the call can't
be interrupted; signals queue oddly), which breaks real workloads (a training job
you Ctrl-C, an orchestrator that SIGTERMs a container, a debugger that SIGSTOPs).

The goal: make a forwarded ioctl behave, from the guest process's point of view,
exactly as a native blocking ioctl does under signals — interruptible when the
underlying nvidia/UVM call is interruptible, with the **guest kernel** owning the
restart-vs-EINTR decision per the guest app's `sigaction`.

## Foundation it builds on
The C-2 fix already established the per-txn pool worker as the unit of execution.
The signal path targets exactly that worker thread, and reuses a txn_id → worker
mapping. So: C-2 (race-correctness) first, signals (semantic-correctness) next —
they compose.

## Flow
1. A signal becomes pending for a guest process while it is blocked in a forwarded
   ioctl. The **guest kernel** (which is running the guest task's syscall) detects
   this and sends QEMU an **"interrupt this transaction"** request keyed by
   (isolate_id, handle_id, txn_id).
2. QEMU forwards the interrupt to the owning isolate/stub.
3. The stub's **main thread** `pthread_kill()`s the worker thread handling that
   txn — but **only if that worker is still on that txn_id** (safeguard: the worker
   may have already completed and moved on; killing it then would hit an unrelated
   txn). Requires a txn_id → worker-tid map in the stub.
4. The delivered signal (a private SIGUSR/SIGINT-class signal, see below) hits the
   worker blocked in `ioctl()`:
   - If the nvidia/UVM call is in an **interruptible** wait, it returns
     `-EINTR`/`-ERESTARTSYS` and the stub's handler runs.
   - If **uninterruptible**, the signal is queued and delivered when the ioctl
     returns on its own → effectively a no-op interrupt (the call completed).
5. The stub reports the ioctl's outcome (EINTR vs completed-with-status) back
   through QEMU to the guest kernel.
6. The **guest kernel** replays native semantics: if EINTR, run the guest app's
   signal handler, then restart or return EINTR **per the guest app's `sigaction`
   SA_RESTART**; if the call completed, deliver the signal after the syscall
   (normal post-syscall delivery). The guest only ever interrupts a guest process
   mid-syscall if the isolate confirmed the interrupt actually landed.

## Four refinements (critical implementation details)
1. **Stub handler installed WITHOUT `SA_RESTART`.** Otherwise the *host* kernel
   auto-restarts the interrupted ioctl, it never returns EINTR to the stub, and the
   guest never learns it was interrupted → the guest app's signal semantics break.
   No SA_RESTART → host ioctl returns EINTR → stub forwards it → **guest kernel owns
   the restart decision** per the *guest* app's sigaction. The restart authority
   must live in the guest, not the host.
2. **SIGSTOP is uncatchable and the stub must never actually stop.** You cannot
   install a handler for SIGSTOP, and stopping the stub thread would wedge a shared
   service (other guests' work). Model SIGSTOP like the interrupt: deliver the
   private break-signal to return EINTR from the ioctl, report to the guest, and let
   the **guest kernel** stop the guest *process*. On SIGCONT the guest re-issues /
   restarts the call. The stub keeps running throughout.
3. **Guest owns restart vs EINTR** (corollary of #1) — based on the guest app's
   sigaction, not the stub's.
4. **Per-txn tid safeguard** — main thread only signals the worker if it is still
   on that txn_id (avoid racing the completion and hitting a later txn).

## Signal choice
Use a dedicated real-time-ish signal (e.g. SIGUSR1 or a chosen SIGRT) as the
"interrupt this ioctl" signal, NOT the guest's actual signal number — the stub only
needs *a* signal to break the host ioctl; the guest signal number/semantics are
replayed guest-side. Handler is installed in the stub before seccomp lockdown; the
handler body does nothing but record state (so it needs no syscalls, or only
allowlisted ones).

## seccomp
The steady-state filter must allow `rt_sigaction`/`rt_sigreturn` (handler install +
return) and `tgkill` (main thread → worker). Handler installed pre-seccomp.

## nvidia interruptibility
Mixed: some RM/UVM waits are interruptible (check `signal_pending`, return
ERESTARTSYS/EINTR), some are uninterruptible. The model handles both — the
"uninterruptible → no-op, call completes" branch is correct and needs no special
case. We do NOT need nvidia to be interruptible for correctness; we need to *not
misreport* an uninterrupted call as interrupted.

## Relationship to KILL
SIGKILL / forced reap is the existing separate path: KILL_ISOLATE is always allowed
immediately, errors all in-flight txns for that isolate back to the guest, then the
kill-success arrives (kill-success ≠ a txn; also fired on stub crash). The guest
then drops the isolate + its on-isolate handle entries. Signal delivery above is for
*catchable/stop* signals during a still-living isolate; KILL is the teardown path.

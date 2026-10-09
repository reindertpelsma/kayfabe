# V3 — the interrupt flood: a debug-only, perturbing experiment

**STATUS: LIVE, 2026-10-09 — DIAGNOSTIC-ONLY, default off, perturbing.** Code and GPU-free tests on
branch `claude/debug-irq-flood-20261009` (on top of `claude/windows-reset-20261009`); the hardware
runs are `traces/windows_reset_20261009/README.md` §15. Never a production configuration and never a
fix: it exists to answer one question and then to be deleted or kept off.

## 1. Purpose

Windows 11 under kf3 shows the lock screen for about 3 s, goes black, and the guest tears its driver
down (TDR 0x116). At that point every Passthrough twin has `GPGet == GPPut` and every semaphore
release is in guest memory, so the GPU work is done and the guest is missing a completion
NOTIFICATION (`windows_reset_20261009/README.md` §13-§13.2). Several interrupt types the VFIO
reference guest takes are rare on kayfabe, most of all the GSP interrupt (vector 155: pending in 24%
of the reference's ISR reads against 0.2% here) and the display head-timing status. The owner's
idea: a thread that periodically fires every interrupt type the guest enabled. If Windows survives,
a missing notification type is the cause and the classes bisect it; if it still dies, interrupts
alone are not the cause.

## 2. The knob

`KF3_DEBUG_IRQ_FLOOD=<classes>:<period_ms>` (one environment variable; a malformed value refuses
realize by name). Classes, comma-separated:

| class | what is raised every period (only if the guest has enabled it) |
|---|---|
| `nonstall` | every non-stall engine vector of the served interrupt table |
| `gsp` | the GSP stall vector (`MC_ENGINE_IDX_GSP` = 50) |
| `disp` | the display stall vector (`MC_ENGINE_IDX_DISP` = 2) |
| `all-completion` | `nonstall` + `gsp` + `disp` |
| `errors` | every OTHER stall vector of the table. Explicit opt-in; **never** part of `all-completion` |
| `dispstat` | raises nothing; makes the display model PRESENT `STAT_HEAD_TIMING(h)` bit 1 (LAST_DATA) set and `INTR_DISPATCH` bit h set while the guest's `RM_INTR_EN_HEAD_TIMING(h)` has that bit set (what `0x611c00`/`0x611ec0` read on hardware). Only the words the guest reads change; no event, frame or interrupt logic does. Separate so its effect can be bisected |

## 3. Rules (checked by the unit tests in `crates/kf-qemu/src/irqflood.rs`)

1. **Its own thread** (`kf3-irqflood`): never a vCPU, never the register drainer, sleeps on the
   monotonic clock between ticks in slices of at most 20 ms and stops when the device stops. It
   serves no input, so a sleep makes nothing deaf (`THE_CONSTRAINTS.md` §35).
2. **Normal path only**: `Device::latch_and_deliver(v)` = the leaf-pending latch plus the irqfd. The
   guest sees an ordinary level-pending leaf bit that it must W1C and service.
3. **Only enabled vectors**, from kf's own record of the guest's `CPU_INTR_LEAF_EN_SET/CLEAR` and
   `TOP_EN` writes (`CpuIntr::vector_enabled`), evaluated at every tick. The class sets come from the
   served kernel interrupt table (`HostFacts::intr_table`), not from a captured one.
4. **No completion word of any kind**: no semaphore, no GP_GET, no notifier record, no POST_EVENT, no
   GSP message. A bare interrupt announces nothing, so it is not a forged completion.
5. **Visible**: the boot log prints `PERTURBING DIAGNOSTIC ON: irq-flood <classes>:<ms>` and the
   status line carries `PERTURBING DIAGNOSTIC ON: irq-flood <classes>:<ms> ticks=N raised[v154=N …]`.
6. A run with it on is a perturbed run: never compare its timings or counts to an unperturbed one
   without saying so.

## 4. Decision tree (stated before the hardware runs)

- **Falsifier.** With `all-completion` at 10 ms the screen still goes black within about 5 s of the
  lock screen appearing and the guest tears down as in the baseline: interrupts alone are not the
  missing piece. What is left: a completion word, a GSP message or event, or status values.
- **Decisive success.** Lock screen persists > 20 s, no bugcheck for 60 s, QGA answers a command.
- **Partial** (lock screen outlasts the baseline's ~3 s, but not decisive): bisect at the period that
  helped: `gsp` only, `disp` (+ `dispstat`) only, `nonstall` only. `errors` only if all three fail
  and a decision is taken to look at error vectors.
- **Better with `gsp` only**: the GSP interrupt rate/shape is the lead; next, when kayfabe raises
  vector 155 relative to its replies and POST_EVENTs.
- **Better only with `disp` + `dispstat`**: the display ISR needs the head-timing status; fix the
  display model's status registers (the `0x611d80` base `0x3f0060` first), not a flood.
- **Better only with `nonstall`**: a notifier edge is lost or late; check the armed-rule relay
  (`OWNER_RULINGS.md` §X).
- A flood that "works" is a lead, not a fix: the fix must come from the same facts, raised by the
  events that really cause them.

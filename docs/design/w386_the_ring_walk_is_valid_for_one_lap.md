# w386 — THE GPFIFO RING WALK IS VALID FOR EXACTLY ONE LAP

**STATUS — 2026-09-06 — LIVE.** Fix merged to master at `65af51eb`, verified at the logic
level. The **hardware arm is PRE-REGISTERED BELOW AND NOT YET RUN**; nothing here claims the
guest bug is fixed.

## 1. The defect

`run_submission` (`crates/kayfabe-rt/src/ceutils.rs`) decided how many GPFIFO entries were new
by **scanning forward until an entry decoded to nothing**. The rationale was written into the
source (`ceutils.rs:87-99`):

> ⊘ Why a cursor rather than a read of `GP_PUT` … this port does not know where this channel's
> USERD lives … an unwritten entry is zero

⊘ **That is sound for exactly ONE LAP.** Once the guest has written every slot once, no zero
entry remains anywhere in the ring, ever again. The `break` stops firing, the walk runs to
`MAX_ENTRIES_PER_DOORBELL = 8`, consumes **7 stale entries** from the previous lap, and the
cursor never re-syncs.

★ **It bites at slot `N-1`, not "after the lap"** — slot `N-1`'s forward neighbour is slot 0,
which lap 1 already wrote. That is the arithmetic behind the measured numbers, not a
correlation.

⚠ **This is not merely a hang.** Past the wrap we re-execute retired `LAUNCH_DMA`s and
**re-release stale semaphore payloads** over the word the guest polls — a completion sent for
work the guest did not submit on that doorbell. It violates the owner's standing rule that a
completion may only be sent when the observed state after it is intended and safe.

⊘ The doc comment at `ceutils.rs:99` asserting *"the cursor is what makes re-execution
impossible"* was **false past the wrap**, and is corrected in the fix.

## 2. What was measured, before any fix (`run_w384c_guest_probe.log`, rev `5756322d`)

| observation | value |
|---|---|
| 64-entry ring | `first_stall_at=63` |
| 32-entry ring (`KAYFABE_LADDER_GPFIFO_ENTRIES=32`) | `first_stall_at=31` |
| wrap bisection `n=63` | `stalled=false`, but `R6 control (close) = Lost { saw: 3735880580 }` = **`0xDEAD0384`**, the untouched sentinel |
| wrap bisection `n=64` | `stalled=true first_stall_at=63` |
| `reps=3`, one channel | rep0 `=63`, **rep1 `=15`, rep2 `=15`** — the FIRST drain window (drains every 16) |
| native (real hardware, walks `GP_GET → GP_PUT`) | six laps, zero stalled drains |

★ The 64/32 readings were **pre-registered before the run**, so *"it dies at absolute index
63"* is **falsified**: the wedge tracks the ring's own modulus.

★★ The `rep1/rep2 = 15` rows are the strongest single corroboration and were not part of the
original account: after rep 0 has written the ring end to end, rep 1 is desynced from its
**first** doorbell, so it breaks at the first window available to it.

## 3. The fix (`65af51eb`)

The walk stops at the guest's own producer cursor: `if idx == gp_put { break; }`, checked
**before** the read, with a two-segment wrapped walk. `MAX_ENTRIES_PER_DOORBELL` survives as
the hostile-ring cap but no longer terminates a normal walk. The zero-terminator remains only
as a secondary safety break.

The shim already read the producer cursor and threw it away — `fb_userd_cursors`
(`shim.rs:16698-16713`) fed a GR observer log line only.

⚠ **A lock hazard that shaped the design:** `RegPlane::fb_peek` takes the plane's state mutex
and `ce_session_with_root` holds that **same non-reentrant `RankedMutex`** across its closure.
Reading `GP_PUT` from inside `run_submission` would deadlock. All call sites read it *before*
opening the session; that ordering is load-bearing.

### 3.1 The unavailable-`GP_PUT` decision: REFUSE BY NAME

`FwdFault::RingProducerCursorUnknown`. Falling back to the zero scan would silently reinstate
this exact bug.

Evidence: every CE channel in the committed boot record (w267 ×8, w269, w274b, w279, w281,
w283, w287) declares its USERD in the **framebuffer** — `phys=fb:0x…/0x200` — with live
cursors (`fbuserd@0x50088 GET=0 PUT=1`, then `GET=1 PUT=1` after the engine fetched). Zero
`phys=sys:`, zero `UNDECLARED/UNREADABLE` in the whole `traces/` tree.

⊘ **What would refute it:** a **Sysmem USERD on a CE channel** is legal, `framebuffer_base()`
correctly returns `None` for it, and nothing in code prevents it. The evidence is an *absence
in the boot record*, not a census over channel shapes. Such a channel would **newly refuse
where it previously served its first lap.** That is the risky half of this change.

An out-of-range cursor is its own fact (`RingProducerCursorOutOfRange`) — never coerced with
`gp_put % entries`.

★ The **observer** (`read_submission_methods`) takes the opposite policy deliberately: bounded
by `GP_PUT` when available, never refused when absent. It releases nothing, so a stale entry
there is a wrong log line; refusing would blind the instrument on exactly the channels whose
failure it describes.

## 4. Logic-level verification (VERIFIED BY ME on `kb`, both commits, one isolated clone)

```
621f257 (RED, test only)  -> FAILED, 9 passed 1 failed, rc=101
c7c2c98 (FIX)             -> 13/13 ok, rc=0
```

The red test's own message names the mechanism:

> doorbell 7 (slot 7) consumed 8 entries; the guest wrote ONE … `CeUtilsRun { entries: 8,
> methods: 56, launches: 8, releases: 0, spans: 8, bytes: 512, completions: 8, cursor:
> GpCursor { next: 7 } }`

★ `completions: 8` = **seven stale semaphore releases in one doorbell**, and `cursor.next`
returning to 7 means it re-walks the *whole ring* on every later doorbell, not merely "7
ahead".

★★ `lap_one_stops_at_the_first_unwritten_entry` — the **control**, the path `cup3`'s
`CUP3_VAL=43` and the 8 GR completions run over — is **GREEN before and after**.

⊘ A test *did* already drive `run_submission` (`tests/tests/e10e_ceutils_doorbell.rs`, since
E10e). What never existed was a test that walks the ring **round**. Note it lives in
`kayfabe-tests`, so `cargo test -p kayfabe-rt` does **not** run it.

## 5. ★★★ PRE-REGISTERED HARDWARE OUTCOMES — written and committed BEFORE the run

The arm: build the QEMU shim at `65af51eb`, boot the Mode-2 guest, run the w384
doorbell-latency rung (`KAYFABE_CE_EXECUTOR=local`, as the failing run used) at `n=64`,
`n=200`, and `--doorbell-latency-reps 3`; plus `cup3` as the known-positive control.

- **(A) FIXED** — `first_stall_at=none` at `n=64` *and* `n=200`, the closing control **lands**
  (not `Lost { saw: 0xDEAD0384 }`), reps 1 and 2 no longer stall at 15, and `CUP3_VAL=43`
  still passes. ⇒ the account and the fix are both right.
- **(B) UNMOVED** — still `first_stall_at=63`. ⇒ the fix is **not on this path**, or the
  account is wrong. ⊘ NOT a tuning problem; do not touch thresholds. Re-open §1.
- **(C) NEWLY REFUSED** — `RingProducerCursorUnknown` appears in the boot log. ⇒ §3.1's named
  risk fired: a channel whose USERD we cannot read now refuses where it previously served its
  first lap. **A different fact from (B)** and it does not share a verdict — the wrap is fixed
  and a *new* refusal is exposed.
- **(D) CONTROL BROKEN** — `CUP3_VAL` is not 43, or the 8 GR completions regress. ⇒ the fix
  disturbed lap one despite the green unit control. Revert first, diagnose second.
- **(E) PARTIAL** — rep 0 clean but reps 1/2 still stall at 15. ⇒ the wrap is fixed and
  cross-rep state is not; the account is incomplete.
- **(F) UNMEASURED_NO_BUILD / UNMEASURED_NO_GUEST** — attributable to the build or the boot,
  **never** to the fix. ⚠ Distinct from every row above and must not be reported as one.

⊘ **Zero bytes in a log is not "still running".** Check process liveness *and* the terminator
line; `143` (job killed) and `124` (launcher timed out, job fine) mean opposite things.

## 6. Residuals, deliberately NOT closed in this rung

- ★ **The same bug class on the `Emulated` forward-ring path** — `device.rs:2890-2900`:
  monotonic cursor, `ring[at..]` linear slice, zero-terminated scan. Gated `Emulated`-only
  (`device.rs:6140-6147`) so it is **not** what bit this guest, but it will bite the first
  kernel `Emulated` CE channel that completes a lap. Its own doc already admits it
  (`gpu.rs:761-764`): *"nothing here handles a guest that fills its ring and wraps to index
  0."*
- ⊘ **`RING_ENTRIES_FALLBACK = 4096`** (`ceutils.rs:78`, `:493-497`, `:826-829`) silently
  substitutes when `chan.ring_entries == 0`. It is a **divide-by-zero guard before it is a
  modulus** (`% entries` panics on a vCPU beneath the BQL), so it cannot simply be deleted.
  Can only fire on a verbatim guest-declared `gpFifoEntries == 0` on a `Ce`-routed channel
  that also declared a ring VA; not present in any boot in `traces/`. Left as-is **on
  purpose**: it is a second behaviour change on the same walk, and the hardware arm
  **cannot grade it** — it would fire on channels this campaign has never observed, so a boot
  could not tell a correct refusal from a new wedge.

## 7. ⊘ The one row the account does NOT fully explain

`n=63`, `R6 control (close) = Lost { saw: 0xDEAD0384 }`. The arithmetic says the control's own
entry (slot 63) *is* consumed first and *should* land.

The candidate reading — **unverified** — is that `0xDEAD_0384` is `DBL_SENTINEL`, which
`fill_words` writes across the whole `W379_BYTES` region at setup, and that one of the seven
**re-executed stale `LAUNCH_DMA`s** in that batch has a destination covering `OFF_POST`,
re-filling the sentinel *over* the control's landed magic.

⚠ If that is right it is **strictly worse** than the diagnosis in §1: not a missing write but
a **retired copy re-running and clobbering a live value**. The fix would remove it either way.
If it is wrong, the account has a hole and **this row is where it is** — which is the thing to
check first if outcome (A) does not come back clean.

## 8. Observability closed alongside

`SERVED-LOCAL` recorded *that* a doorbell was served locally but **not how many ring entries it
consumed** — the exact quantity this defect corrupts — and was print-capped at 16 lines per
boot. With that number logged, the defect would have been visible as `entries=8` in any boot
log instead of inferred from stall positions.

New line: `CE-SERVED-LOCAL[ ⚠ODD] #n token=… chan=… entries=N gp=A->B gp_put=P ring_entries=E`.

★★ **Two per-channel budgets, not one.** Ordinary rows (1 entry, no wrap) and *remarkable*
rows (`entries != 1` **or** the walk wrapped) have separate budgets. A single cap would have
hidden exactly the rows the defect produces — a 1024-entry ring wraps at doorbell 1023, a
thousand ordinary rows after any budget is spent. Same class as the global cap that forged an
absence in w383. `gp=7->7` while `entries=8` is the signature.

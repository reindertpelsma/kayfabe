# ★★★★★ w288n — THE GOAL METRIC MOVED. `CUP2_RC` 124 → **1**, and the notifier path fired 22×.

`[measured 2026-08-13, vh, RTX 3060 GA106 / 580.159.04, rev aea02a52, stamp gate PASS]`
Baseline for comparison: `CUP2_RC=124` at `rev 54af1d7d`, same six arms, pre-registered.

## The number

```
--- ★★★★★ CUP2_RC = [CUP2_RC=1]
```

Anchored. **6/6 carried arms PASS** (`FB-JOIN=shared · GUEST-RING=ring · GUEST-PUSHBUF=pin ·
GUEST-SEMA=pin · GR-ROUTE=passthrough · GUEST-OPERAND=pin`) — the only thing that differs from
the 124 baseline is the code under test.

★★ **THE ANCHOR TRAP FIRED AGAIN**: `unanchored, for contrast: [CUP2_RC=0 CUP2_RC=1]`. An
unanchored reader would have reported `0` on this run too.

## ★★★★★ THIS IS THE PRE-REGISTERED SHAPE, AND IT MUST NOT BE OVER-READ

The owner registered the expectation before the run: *if the hypothesis is right this converts
**a hang into a REPORTED FAILURE**, not into success.* That is exactly what happened.

- `124` is the harness **timeout** — cup2 hung and was killed.
- `1` is **cup2's own exit code** — it ran to a decision and reported failure.
- cup2's last prints are unchanged (`totalMem=11959 MiB`), so it still dies at `cuCtxCreate`.

⇒ **The UVM-notifier hypothesis is CONFIRMED.** `uvm_channel_get_status` is UVM's only error
exit; nothing wrote slot 0, so the channel waited forever. Now the host RM/GSP writes the
guest's own notifier page and the guest **leaves the loop and reports**.
⊘ **The GR fault is NOT fixed and was never going to be.** Telling the guest about a real fault
does not make the fault go away. The fault itself is the next rung.

## The path demonstrably RAN — this is not a coincidental change

```
ERROR-NOTIFIER built   = [22]
ERROR-NOTIFIER REFUSED = [0]
```

⊘ The known-negative at the baseline revision was **0** (`h_object_error` hard-coded `0` at
`rm.rs:5095`/`:5136`), asserted in `traces/boots/w288n/RESULT.md` precisely so that a non-zero
count here is a **CHANGE** and not a first sighting. **Zero refusals** — every channel that
declared a sysmem notifier got one, so `NotifierGpaMisaligned` (the arm most likely to silence
this rung) did **not** fire on a real guest.

## The fault, by IDENTITY — still there, as expected

```
NVRM: Xid 31, name=memfd:kayfabe-i, channel 0x00000009,
  MMU Fault: ENGINE GRAPHICS HUBCLIENT_FE faulted @ 0x7107_c6e00000.
  Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

Same class as the baseline's (`@ 0x72a5_fee00000`) and w273's (`@ 0x75b2_aee00000`) — the VA
moves per boot, as unified addressing predicts (the GPU VA *is* the process VA).

## The negative controls

- **Native, no deliberate fault** (`--ce-client --notifier-vidmem`):
  `NATIVE_NOFAULT_XID_DELTA=95->95` — **nothing faulted, nothing reported.**
- **Native, deliberate fault** (`--ce-client-fault --notifier-vidmem`):
  `NATIVE_FAULT_XID_DELTA=95->96` — the control **firing**, one Xid, as designed.
- **Guest, no deliberate fault** (boot 2 ran `--ce-client`): `ERROR-NOTIFIER built = 2` and no
  client fault. ⇒ a guest-side negative control, arrived at by accident (see the defect below).

## ⊘⊘ TWO DEFECTS IN THIS RUNNER, NAMED RATHER THAN QUIETLY FIXED

1. **Criterion 1's guest FAULT arm did not run.** `r33_hook_ce_client.sh:47` is
   `ARGS=${KAYFABE_R33_ARGS:---ce-client}` and the runner never set `KAYFABE_R33_ARGS`, so boot
   2 ran the **no-fault** arm. Its output says so in its own words: `R33 arm 4 = NOT RUN (pass
   --ce-client-fault)`. ⇒ It is a valid negative control and it is **not** criterion 1. Re-run
   separately as `w288nc1`.
   ⚠ Exactly the class this campaign keeps paying for: *an arm that reads as run because a boot
   happened*. Only the client's own "NOT RUN" line distinguishes it.
2. **The native arms' verdict lines were truncated** by `| tail -30` / `| tail -40`: the ioctl
   census is longer than that, so it displaced the `R33 arm 5` verdicts. The `XID_DELTA` pair
   above survived and carries the control result; the per-field identity did not.

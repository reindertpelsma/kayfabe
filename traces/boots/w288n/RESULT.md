# w288n — cup2 BASELINE on the passthrough build. `CUP2_RC=124`, as pre-registered.

`[measured 2026-08-13, vh, RTX 3060 GA106 / 580.159.04, rev 54af1d7d (stamp gate PASS)]`

## The number

```
--- ★★★★★ CUP2_RC = [CUP2_RC=124]
```

Anchored. All six carried arms asserted out of the device's own lines:
`FB-JOIN=shared · GUEST-RING=ring · GUEST-PUSHBUF=pin · GUEST-SEMA=pin · GR-ROUTE=passthrough ·
GUEST-OPERAND=pin` — **6/6 PASS**, so this is comparable to the w287 passthrough boots.

★ **Pre-registered before the run** as `124`, because this revision contains none of the
error-notifier work. It is a BASELINE, not a test of anything this rung designs. Its value is
that **cup2 had never been run on the passthrough arming at all** — `w287_run.sh` says so in its
own header, and the `GP_GET 0→1` result everyone quotes was the RAW CLIENT's channel.

## ★★★★★ THE ANCHOR TRAP FIRED LIVE ON THIS RUN — keep it

```
unanchored, for contrast: [CUP2_RC=0 CUP2_RC=124 ]
```

An unanchored `grep -o 'CUP2_RC=[0-9]*'` matches `GCC_CUP2_RC=0`, the guest COMPILER's status,
**first**. ⇒ On this very run an unanchored reader would have reported **`CUP2_RC=0` — the
campaign's headline success value — on a hanging arm.** The banked trap is not historical.

## The fault, by IDENTITY (not a count)

```
NVRM: Xid (PCI:0000:00:07): 31, pid=2438057, name=memfd:kayfabe-i, channel 0x00000009,
  MMU Fault: ENGINE GRAPHICS HUBCLIENT_FE faulted @ 0x72a5_fee00000.
  Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

Same class as w273's GR fault (`HUBCLIENT_FE`, `FAULT_PDE`); the **VA differs per boot**
(`0x72a5_fee00000` here, `0x75b2_aee00000` in w273), which is what the unified-addressing finding
predicts — the GPU VA *is* the process VA, so it moves with the guest process's ASLR.
⚠ Note `ACCESS_TYPE_VIRT_WRITE`. The faulting process is `memfd:kayfabe-i` — **the isolate**.

## cup2's own last prints — it dies exactly where it always has

```
ok   cuDeviceTotalMem(&tot,d)
totalMem=11959 MiB
CUP2_RC=124
```

`totalMem=` is cup2's last print before `cuCtxCreate`. The wall is unmoved.

## The known-negative, asserted rather than assumed

```
h_object_error census lines = [0]
```

The error-notifier surface is **absent at this revision** — as it must be, since `h_object_error`
is hard-coded `0` on both host objects (`rm.rs:5095`, `rm.rs:5136`). ⊘ Recorded so that a later
run showing a non-zero count is a CHANGE and not a first sighting.

## Guest driver evidence persisted (the banked serial-log trap)

`run_w288n_guest_dmesg.log`: **31 NVRM lines, 0 adapter errors**, asserted non-empty. The residual
`0x56` statuses are the known forgiven `NV_ERR_NOT_SUPPORTED` family, unchanged.

## ⊘ WHAT THIS RUN CANNOT SAY

Nothing here bounds the notifier design. It does not test the host-RM-object-over-guest-pages
route, which is not in this binary. A `124` here is **not** evidence against that design.

# w305 RESULT — the PREEMPT destroy path is OBSERVED, and criterion 1's diagnosis is REFUTED

`[measured 2026-08-14, vh2, real GA106, driver 580.159.04, rev c7c058a3]`
Bench tree `/workspace/kayfabe_w305`, clean, stamp gate passed on every boot.

| run | what | outcome |
|---|---|---|
| `w305_native_fault.out` | rmladder `--ce-client-fault`, **native, no QEMU** | ★ known-positive, criterion-1 machinery fully works |
| `w305apreempt` | `cup3d` (cup3 + explicit `cuCtxDestroy`) | ★★★★★ **item A outcome (a)** |
| `w305bfresh` | rmladder `--ce-client-fault` **in the guest** | **item B outcome (c)** |
| `w305bshared` | rmladder `--ce-client-fault-shared-vas` in the guest | outcome (d) — the arm did not run |

---

## 1. ITEM A — outcome (a). THE CONTROL ARRIVES ON A SUCCESSFUL DESTROY, AND IT IS REFUSED

`w303` shipped `respond_preempt` and named its own gap: *"a refusal on the successful destroy
path has never been observed"*, because `cup3.c` never calls `cuCtxDestroy`. `cup3d.c` is cup3
byte-for-byte plus an explicit teardown.

**The compute leg still crosses** — so this is a destroy-path measurement, not a broken run:

```
KERNEL rv=43 want=43 -> PASS      ^CUP3_VAL=43   ^CUP3_RC=0
```

**The control arrived exactly once, and took the REFUSAL branch:**

```
kayfabe: PREEMPT client=0xc1d0000c object=0x5c000012 proc=2 → ⊘ UNPERFORMED
host_twins=8 of materialized=8 (unmaterialized=0) ⇒ this group has a LIVE host twin on real
silicon and this port has no preempt verb. Answered 0x40 NV_ERR_INVALID_STATE — inside
ctrla06c.h's own status set — rather than NV_OK.
```

| branch | count |
|---|---|
| `★ NV_OK, AND IT IS TRUE` (no host twin) | **0** |
| `⊘ UNPERFORMED` (live twin → `0x40`) | **1** |
| `⊘ UNROUTABLE` | 0 |
| `⊘ REFUSED BadParams` | 0 |

⇒ **The branch that had never been observed on a successful destroy is the one that fires**,
and it fires with `host_twins=8 of 8` — every member of the group materialized on real silicon.
The `NV_OK`-no-twin arm is **still unobserved on this path**; a boot where it fires would need a
group that never submitted.

### ★★★ AND THE GUEST ACCEPTED IT — this is NOT outcome (c)

```
DESTROY_MEMFREE_IN_RC=0  DESTROY_MEMFREE_OUT_RC=0  DESTROY_MODUNLOAD_RC=0
CUP3D_CTXDESTROY_RC=0    CUP3D_CTXDESTROY_STR=no error
TEARDOWN DONE
```

`cuCtxDestroy` returned `CUDA_SUCCESS` **despite** our `NV_ERR_INVALID_STATE`. libcuda does not
propagate the refusal. ⇒ **w303 introduced no regression on the successful destroy path**, and
refusing costs the ladder nothing — now measured on the path itself rather than inferred from
boots where `cuCtxCreate` had already failed.

⚠ **What this does NOT say.** The guest freed the pages anyway. Our answer is *honest* (we said
we did not preempt) but the **free-after-ring hazard is not closed** — w303's own scope note
stands: *"this does not preempt anything"*. The value here is that the guest is no longer told a
false postcondition, not that the postcondition is now true.

**Known-positive for the census** (without it a zero would be vacuous): siblings `0xa06c010a`,
`0xa06c0101`, `0xa06c0103` each appear **1×** in this boot. The census is live.
**Regression check:** `Xid count = 0`, `host_rows=18295 of 18309` — cup2/cup3's established
address-plane values, unmoved.

---

## 2. ITEM B — outcome (c). `CONTROL-NEVER-LANDED` REPRODUCES, AND §2'S FIX WAS A NO-OP

### 2.1 The prescribed fix does not exist

`road_to_v1_after_cup2.md` §2: *"The fix is one line in the probe: allocate its control operands
in the VAS of the channel it rings."* **They were never anywhere else:**

- `probe_guest_reachability(vas, …)` takes `range = self.narrow(vas)` (`rm.rs:6976`)
- maps **every** operand into it — `ctrl_src` (`:7036`), `dst` (`:7044`)
- creates the ringing channel on **the same `vas`** (`:7080`)
- and `narrow` is `u32::try_from(h.raw())` (`rm.rs:4233`) — a pure handle-width cast, not a
  re-derivation.

### 2.2 The diagnosis is refuted by a native known-positive

Same binary, same `--ce-client-fault`, same **third freshly-allocated VAS**, run natively:

```
info  R33 arm 4 SPACE  = a THIRD, freshly allocated address space (range 0xcafe0011)
★     R33 CRIT1 STATE  = FAULT-PROVOKED-ADDRESS-READ | VA-IDENTITY MEASURED = yes
★     R33 arm 5 WHERE  = GET_MMU_FAULT_INFO addr=0x0000000900000000 faultType=0x0
                         faultString="FAULT_PDE" | VA-IDENTITY HOLDS
host: Xid 31 … channel 0x00000005 … MMU Fault: ENGINE CE0 HUBCLIENT_CE1
      faulted @ 0x9_00000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
```

⇒ On real hardware that exact arrangement provokes the fault, reads its address, and **two
independent observers — the client's own in-process `GET_MMU_FAULT_INFO` and the host kernel
log — name the same address.** A fresh VAS blocks nothing. **The probe is correct.**

### 2.3 The guest arm — outcome (c)

```
★  R33 CRIT1 STATE = CONTROL-NEVER-LANDED | VA-IDENTITY MEASURED = no
?? R33 arm 4 control = the POSITIVE CONTROL did not land
   (sem 0x00000000, GP_GET 1 GP_PUT 1, moved 0xdead0000 want 0x5ea1c071)
   operands: src 0x700100000 dst 0x700200000 ring 0x700000000
```

Host `Xid` count **0**; guest `dmesg` carries **no `Xid`** either. The cursors **caught up** and
the sentinel **survived** ⇒ the ring was consumed and no bytes moved. Doorbells routed
(`by engine: … Ce=18 … unrouted=0`; 25 `engine=Ce`, 4 `engine=GrGraphics`).

⇒ Criterion 1 is **still unmeasured**, and the blocker is our CE plane carrying this channel —
not the probe's VAS choice.

### 2.4 ★★★★★ THE DISCRIMINATOR THE RULING WAS REACHING FOR — in the same boot

| arm | VAS | result |
|---|---|---|
| **1** | `vas`, allocated first, **already carried work** | ★ **4096 bytes moved**, semaphore `0x1` = declared, `GP_GET 1` caught `GP_PUT 1` — the whole four-fact bar, **in the guest** |
| **4** | `fvas`, **freshly allocated**, same engine `COPY0` | ⊘ control never landed, nothing moved |

⇒ **The VAS is a live variable in the guest after all** — but not for the reason §2 gave. The
difference is *a VAS that has already carried retired work* vs *one that has not*, not *operands
in the wrong VAS*. ⊘ Natively **both** work, so the asymmetry is ours.

⚠ **NOT yet isolated.** Arm 4 also differs from arm 1 by being the **second** channel in the
process, by dictating its ring at `0x7_0000_0000`, and by carrying an error notifier.

### 2.5 The isolating arm did not run — outcome (d), reported not smoothed

`--ce-client-fault-shared-vas` runs arm 4 in arm 1's own VAS. It returns:

```
★    R33 CRIT1 STATE = PROBE-NOT-BUILT
FAIL R33 arm 4 = the probe could not be built:
     BadHandle(HostHandle(iso0/gpu0:0xcafe0005))
```

Cause is **our bookkeeping, not RM**: `alloc_channel_in` resolves `self.conn.space_of(range)`
(`rm.rs:5831-5833`), and arm 1's `vas` handle is the **space** (`0xcafe0005`) whose paired
**range** is a different handle (`0xcafe0009` — the number arms 2/3 print). ⇒ **this arm says
nothing about the VAS hypothesis.** The fix is to pass the range, not the space; it is one line
and it is the obvious next rung.

---

## 3. TWO INSTRUMENT DEFECTS THIS RUNG FOUND IN ITSELF

1. ★★★ **THE ANCHOR TRAP, INVERTED — a FALSE "UNMEASURED" on a measured field.**
   `w305a_preempt.sh` read `grep -oE '^CUP3D_CTXDESTROY_RC='`. `cup3_hook.sh` prints the
   workload's output through `sed 's/^/    /'`, so `^` can never match. The grading block
   printed **"⊘ ABSENT — cuCtxDestroy never returned a line. UNMEASURED, NOT 0"** while its own
   verbatim block six lines above printed `CUP3D_CTXDESTROY_RC=0`. **One log, both claims.**
   ⇒ This tree's standing rule is *anchor, because the unanchored read printed the headline
   success value on a failing arm*. Here the anchor produced the **mirror** failure — and
   "unmeasured" is the reading this repo treats as **safe**, so it would have been believed.
   ★ **An anchor is only correct against the layout the PRODUCER emits.** Two producers write
   into one probe log (the hook at column 0, the workload indented); one anchor cannot serve
   both. Fixed to allow leading whitespace, with the strict read printed beside it.

2. ⊘ **A PROVENANCE LINE THAT WAS TEMPLATE TEXT.** `cup3_hook.sh` printed *"byte-identical to
   the C artifact's tests/mode2/cup3.c (md5 3c90b0f5…)"* **unconditionally** — three lines under
   a source line reporting `md5 67f93d72…` for `cup3d.c`. A provenance claim that is not a
   comparison asserts nothing and is worse than silence, because it reads as verified. Now an
   actual comparison, printed either way.

⚠ Minor, unfixed: `w305b_crit1.sh`'s *"the ARM ACTUALLY PASSED to the client"* field greps the
probe log for `ce-client-fault` and printed `[]` on both boots — the hook never echoes its args.
The arm is nonetheless proved to have run by arm 4's own `SPACE` line, which differs between the
two boots exactly as the flag dictates.

# w297 — cup3, FIRST COMPUTE. `^CUP3_VAL=43` on a stock guest driver, real GA106.

> **STATUS: LIVE — 2026-08-14.** One boot, graded against outcomes pre-registered in
> `scripts/bench/w297_cup3.sh`'s header before the boot. Verdict **(A)**.
> ⊘ This is a **RELAXED** green — every relaxation that was on is named in §4, and the
> milestone is not reached until they come off.

---

## 1. The metric, verbatim and anchored

```
--- ★★★★★ CUP3_VAL=43
--- ★★★★  CUP3_RC=0
    the kernel line, verbatim = [CUP3_KERNEL_LINE=KERNEL rv=43 want=43 -> PASS]
    UNANCHORED, for contrast  = [CUP3_RC=0 CUP3_RC=0 ]

=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary
    (A) ★★★★★ FIRST COMPUTE. 43 cannot be copied, filled, or forged.
```

**`out = in*3 + 1`, `in = 14`, `out = 43`.** No copy engine, no fill, no forged completion and
no emulator in this stack can produce 43 from 14: the CE copies and memsets, the emulator has
no arithmetic, and a forged completion carries a payload *we* chose. ⇒ **the host GA106 GR
engine executed the guest's JITed PTX shader.**

⚠ Note the contrast line: the *unanchored* read returns `CUP3_RC=0 CUP3_RC=0` — one of those
is `GCC_CUP3_RC=0`, the compiler's exit status. That is the anchor trap that fired on seven
consecutive cup2 rungs, and it would have reported the headline value on a failing arm. **The
anchored `^CUP3_VAL=` is the metric.**

## 2. The stage ladder — no ✘ anywhere

```
✔ CTX OK   ✔ MODULE OK   ✔ FUNC OK   ✔ MEMALLOC
✔ LAUNCH OK   ✔ SYNC OK   ✔ KERNEL rv=   ✔ DONE
```

cup3's own `FAIL` line: **absent** — it never named a failure. `CUP3_OUT_BYTES=501`,
`MEMALLOC in=0x7bc234200000 out=0x7bc234200200`, guest exit `0`, `HOOK_RC=0`.

**Attribution precondition met:** `CUP3_JIT_PRESENT=yes`
(`libnvidia-ptxjitcompiler.so.580.159.04` present in the guest), so outcome **(F)** is ruled
out — a MODULE-stage failure would have been ours. It did not fail.

## 3. (E) Regression check — printed even on a green

| axis | cup2 established | w297cup3 | verdict |
|---|---|---|---|
| `Xid` | 0 | **0** | no regression |
| `host_rows` (final) | `18295 of 18309` | **`18295 of 18309`** | **identical** |
| `unrouted` doorbells | 0 | **0** | no regression |
| named refusals | 2 `AllocClassNotPermitted`, 1 `ReservedClient`, 1 `UnmappedAllocClass` | **identical set and counts** | no regression |

⚠ **How the `Xid = 0` is known, because a 0-byte file is exactly the ambiguous artefact this
tree keeps paying for.** `run_w297cup3_hostdmesg.log` is **0 bytes** — and that is the *normal
green*, not a failed capture: it is a per-boot **delta**, and the probe log states the
watermark independently as `HOST dmesg delta for this boot (watermark 1107 → 1107)`,
`HOST_DMESG_LINES=0  HOST_DMESG_NVRM=0  HOST_DMESG_XID=0`. Cross-checked a third way from
outside the harness: host uptime at grading was `492554 s` and the newest Xid in the host's
cumulative `dmesg` is at `457691 s` — **9.7 hours before this boot began.** The QEMU log
likewise contains **0** occurrences of `Xid` or `MMU Fault`.
⇒ The empty file is committed **deliberately as the measurement**; it is not evidence of
absence on its own, and the three independent corroborations above are why it may be read as
one here. (It is stored as an empty file rather than omitted so a future reader sees the
delta was *taken*, not skipped.)

## 4. ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone

Byte for byte w294's cup2 arming; **nothing in the device moved this rung** (`w297_cup3.sh`
invokes `w290p_run.sh drain` rather than copying it, so the two rungs cannot drift). Changing
the workload *and* the arming in one step would have made this outcome unattributable.

| variable | value | |
|---|---|---|
| `KAYFABE_VAS_PUBLISH` | `drain` | ★ the w292 rung |
| `KAYFABE_PT_SWEEP` | `on` | ⊘ **RELAXATION 1** (carried) |
| `KAYFABE_OPERAND_JOIN` | `join` | ⊘ **RELAXATION 2** (carried) |
| `KAYFABE_FB_JOIN` | `shared` | |
| `KAYFABE_GR_ROUTE` | `passthrough` | |
| `KAYFABE_GUEST_RING` | `ring` | |
| `KAYFABE_GUEST_PUSHBUF` | `pin` | |
| `KAYFABE_GUEST_SEMA` | `pin` | |
| `KAYFABE_GUEST_OPERAND` | `pin` | |
| `KAYFABE_PT_WITNESS_EXEC` | `on` | |
| `KAYFABE_ISOLATES` | `real` | |
| `KAYFABE_CE_EXECUTOR` | `host` | |
| `NVKVM_RAM_BACKEND` / `KAYFABE_GUEST_RAM` | `memfd` | |
| `KAYFABE_RING_VIDMEM` | *unset* | |

**Confirmed IN FORCE from the device's own emissions**, not merely from the environment
(a boot happening is not an arm running):
`VAS-PUBLISH arm=drain fb_join=shared host_isolates=true`, `OPERAND-JOIN arm=join`,
`PT-SWEEP tasks=2 skipped=2 ran=2`.

## 5. The unserviced control ledger — ⊘ ZERO cup3-specific demands

**40 distinct `unserviced fn 76 cmd` ids — the identical SET to w294's cup2 baseline (40).**
`comm` both ways returns empty. Against the later w295 cup2 boot (41 ids), w297 lacks only
`0x83de0309`, which w295 *added*; nothing is new here.

⇒ ★★★ **Module load and kernel launch demanded no control this stack refuses.** The
pre-registered "most likely shape of a new wall" — a cup3-specific id in the ledger — **did
not appear**. Everything `cuModuleLoadData` + `cuLaunchKernel` needed beyond cup2 was already
served by what cup2 required.

## 6. Doorbells — where the extra work actually showed up

| | w294cup2 | w295cup2 | **w297cup3** |
|---|---|---|---|
| `by engine:` summary | `GrCompute=119 Ce=358` | `GrCompute=119 Ce=358` | **`GrCompute=125 Ce=355`** |
| per-doorbell `engine=` tally | 827 GrCompute / 686 Ce / 4 GrGraphics | 833 / 686 / 4 | **870 / 686 / 4** |
| `unrouted` | 0 | 0 | **0** |

⇒ **+6 routed GrCompute doorbells** over cup2, `GrGraphics=0`, `unrouted=0`. The launch is GR
compute work and it routed. (`Ce` moves 358→355 — three fewer; cup3's HtoD/DtoH are 4-byte
transfers that the compute class's I2M path can carry, consistent with
`docs/reference/native_dataplane_cup2_ga106.md`.)

## 7. Provenance

- **Source revision: `0655e6aa2c72bfa9528fb0d753b85d039a95c660`** — a fresh clone at
  `/workspace/kayfabe_w297` on the bench, `git status --porcelain` **empty**.
- **Stamp gate PASSED:** `STAMP=0655e6aa… HEAD=0655e6aa…` — the binary that ran is this
  revision. (⊘ `BUILD_REV.txt` says `5feac909…`; it is explicitly *informational, NOT
  authoritative*, and the string embedded in the binary is what the gate compares.)
- Host GA106, driver `580.159.04`, guest driver stock and unpatched. Boot
  `2026-08-14T08:37:14Z`, graded `08:39:12Z`, `W297 EXIT rc=0`.
- ⚠ `nvidia-smi` still prints `ERR!` in the Name column — the known
  `GPU_GET_NAME_STRING` gap, unchanged and unrelated.
- Guest dmesg carries the usual teardown `NV_ERR_NOT_SUPPORTED (0x56)` run
  (`NVA06F_CTRL_CMD_STOP_CHANNEL`, `NV2080_CTRL_CMD_GPU_EVICT_CTX`, fault-buffer
  unregister) at `t≈83–86 s` — **after** `DONE`, on the shutdown path, identical in kind to
  cup2's.

## 8. ⊘ What this run does NOT establish

- It is a **1×1×1 launch of a 6-instruction shader**. It proves the GR engine executed guest
  code and returned a correct arithmetic result; it says nothing about occupancy, multi-block
  dispatch, shared memory, barriers, or throughput.
- It is **one boot**. cup2 was confirmed 2/2 before it was believed; cup3 has not been.
- The relaxations in §4 are still on. ★ **Removing them one at a time, re-running cup3, and
  seeing which one the 43 survives is the natural next rung** — it is now a
  *known-positive* that can grade them, which is a strictly better instrument than cup2 was.

## 9. Files

| file | what it is |
|---|---|
| `w297cup3_harness.log` | `/workspace/w297cup3.log` — the full runner + grading block |
| `run_w297cup3_probe.log` | the hook: JIT precondition, build, stage ladder, the anchored `^CUP3_VAL=43`, guest dmesg |
| `run_w297cup3_qemu.log.gz` | the device log, gzipped (4.9 MB → 150 KB); ledger, doorbells, `host_rows`, arming |
| `run_w297cup3_dmesg.log` | guest dmesg |
| `run_w297cup3_serial.log` | guest serial (⊘ contains **no** driver output — the driver is `modprobe`d over ssh after boot; this file is here to be *checked*, not trusted) |

⊘ `run_w297cup3_hostdmesg.log` is 0 bytes and is **not** committed as a file; §3 states the
measurement and its three corroborations instead, because an empty file in a trace directory
reads as a failed capture to every future reader.

# w330 — THE BENCH IS REBUILT ON FRESH GA106, AND ALL THREE DEFAULT-OFF FLAGS EARN THEIR FLIP

**STATUS: LIVE — measured 2026-08-19**, instance `48097794`, RTX 3060 **GA106**, host driver
**NVIDIA Open Kernel Module 580.159.04**, guest kernel 6.8.0-137, QEMU 10.2.4 + QOM shim,
both artefacts stamped `kayfabe-rev:1fc90253…` == master HEAD.

## 1. Bring-up, ~1 hour from a bare rented box

Verified on CONTENT at every step, never on an exit code:

| step | the check that was actually made |
|---|---|
| die | `lspci` names **GA106** — the die w318/w321 were measured on |
| host driver | `/proc/driver/nvidia/version` says *Open Kernel Module 580.159.04*; `modinfo` `license: Dual MIT/GPL`; **no tainted modules** |
| device nodes | a C probe **`open()`s all three** — `OPENPROBE_BAD=0`. `ls` cannot tell you `RmInitAdapter` ran |
| shim + QEMU | **both** carry `kayfabe-rev:1fc90253…` |
| guest | stock 580.159.04 built **in-guest from the same `.run` the host used**; `cuda.h` verified `grep -c CUresult`=601, **not** by existence (the guest carries three decoy `cuda.h`, all the PowerMac ADB header) |
| bench boot | driver loads, `nvidia-smi` enumerates, `SMI_RC=0`, host Xid 0 |
| **cup3** | `^CUP3_VAL=43` |
| **cup8** | `CUP8 RESULT N=2048 bad=0 maxerr=0 C[0]=2048(exp 2048) -> PASS` |

★ **The driver was deliberately downgraded 580.178.04 → 580.159.04.** Every recorded green boot
used 580.159.04 and the vendored ogkm oracle is pinned there. A baseline on a driver no recorded
boot used would make a red uninterpretable — the confound `nvkvm-pv` paid an afternoon for.

⚠ **Deltas from the recorded config, stated rather than smoothed over:** guest kernel is
**6.8.0-137**, not the recorded -136 (noble moved; nothing pins it). The bench is **nested** —
w315 §4 bounds that tax at ≤ 2.2 ms/launch, which was 2.6 % of an 85 ms trap and is **up to 55 %
of a 4 ms one**. Bare metal matters more now than it did, *because* w318 worked.

## 2. ★★★★★ RECLAMATION — and the shipped default is INERT, not merely weaker

Same binary, arms **interleaved** (SUP, DEF, SUP, DEF), `KAYFABE_BENCH_BW=28,31`:

| | `supersede` ×2 | default (`arm=on`, leg 1) ×2 |
|---|---|---|
| `BWITER mib=31` rows | **7** | **0** |
| `already joined` | **0** | **32** |
| host Xid | 0 | **1** |
| `supersede` events | **279** | 0 |
| `revoked=` / `released=` (leg 1) | **0 / 0** | **0 / 0** |

The default arm's host dmesg:
> `Xid 31 … MMU Fault: ENGINE GRAPHICS GPC1 … faulted @ 0x733f_0ec17000. FAULT_PDE`

★ and `0x733f_0ec17000` is **inside that row's own `in_ptr=0x733f0e000000`**.

⇒ **Leg 1 fires ZERO times on both arms.** The shipped default is behaviourally identical to
`off`; the only thing that acts is the takeover. That is w329's finding confirmed from the other
side — **CUDA's suballocator never unmaps on `cuMemFree`**, so an unmap trigger has no event.
⚠ `revoked=0` is not reassurance; on the failing arm it is the disease.

⊘ **cup8 green cannot close this gate** — its buffers are 16 MiB and their VA never moves, so it
never exercises the path. An LLM is nothing but allocation history.

## 3. ⊘⊘ THE GRADER SAYS PASS ON THE ARM THAT DIED

`GUEST_BENCH_VERDICT=PASS (every bw row verified)`, `TOTAL_BAD=0`, `HOOK_RC=0` — on the run whose
31 MiB row printed `BW_BEGIN` and then **zero iterations**.

**A universal quantifier over an empty set is vacuously true.** A row that dies before its first
iteration contributes no counterexample. `TOTAL_BAD=0` is the same shape: `bad` counts
comparisons that *ran*.
⇒ Every discriminating instrument was a **COUNT**; the verdict string discriminated nothing.
⊘ And `GUEST_XID_COUNT=0` sat beside a host Xid — two counters named `XID`, two different planes.

## 4. ★★★★★ THE TWO PERF FLAGS ACT ON DIFFERENT STATISTICS

`KFTIME mmio_doorbell total_us`, first 400 events, one binary, interleaved:

| arm | median | p90 | max |
|---|---|---|---|
| CTL ×2 | 18 741 / 19 286 | 86 104 / 82 488 | **2 753 760 / 2 726 516** |
| **GATE** | **2 197** (8.5×) | **4 431** (19.4×) | 2 815 676 ⊘ **unchanged** |
| **COAL** | 30 311 ⊘ **worse** | 76 670 | **217 190** (12.7×) |
| **BOTH** ×2 | 9 016 / 11 759 | **10 712 / 14 807** | **249 118 / 394 035** |

★ GATE's p90 **86.1 → 4.43 ms** reproduces w318's **85.248 → 4.078 ms** on different hardware,
driver build and guest kernel.

> ⊘ **Graded on the median alone the coalescer reads as a 1.6× REGRESSION.** Graded on the max
> alone the gate does nothing. Only both statistics separate them.

⇒ Gate removes the per-doorbell **page-table + publication** pass (typical trap); coalescer
removes the guest-RAM **pin** drain (worst trap). **Neither is sufficient alone.**
⚠ n=2/arm. CTL repeats agree tightly; BOTH repeats agree only to ~40 %. Max and median are not
the same event class — there is no single "speedup".

## 5. ⊘ THE `0x110094` HYPOTHESIS IS REFUTED, AND THE CENSUS NAMES OURS

The C's LLM bottleneck was `NV_PFALCON_FALCON_DEBUGINFO` busy-polled at ~99 % of BAR0 reads,
1–3 k vmexits/token. **Here it appears ONCE** (`grep -c "off=0x110094"` → `1`); the whole GSP
falcon page is single-digit. **We do not pay the C's tax**, and read-native/write-trap is not on
our path.

★ The same census names ours:
```
KFTIME-HOTREG bar=0 off=0xbb0090 n=483 share=100.0% total_ms=12278.300 mean_us=25420
```
**One register, 100.0 % of hot-register time, 12.28 s, 25.4 ms mean.** Every other row is
`share=0.0% mean_us=1`. 483 events matches `mmio_doorbell` exactly ⇒ the same population, seen
from the register side. Two independent instruments, one conclusion: **the whole hot-register
cost is our own doorbell handler.**

⊘ **This cost ZERO boots** — the census was already in logs captured for something else.

## 6. Recommendation

**Flip all three.** `KAYFABE_JOIN_RELEASE=supersede` (correctness — the current default is inert),
`KAYFABE_DIRTY_GATE_PUBLISH`/`_WITNESS`, `KAYFABE_DRAIN_BATCH=coalesce`.
`^CUP3_VAL=43` holds on all 8 perf arms including both gates armed.

⚠ **Still open**: `supersede`'s winner-choice between two live aliases remains unproven and
`SUPERSEDE CAPPED` was load-bearing at 266/588 — flipping the default does not retire that.

## 7. Harness defects found (all mine, all the same shape)

1. `EXIT_STATUS=$?` read after an `echo` reported **success over three failed installs**.
2. `pgrep -f '[d]rvswap'` said STILL RUNNING on a finished job — **the bracket trick is not
   sufficient**: a later word on the same command line (`/root/drvswap.log`) matches. Patched into
   the C repo's `CLAUDE.md`.
3. The two prescribed stamp greps **fail in opposite directions**: `[0-9a-f]{40}` is blind to an
   `unknown` stamp; `[[:graph:]]*` **over-captures in a linked binary** (adjacent `.rodata`
   literals, no NUL) and failed a correct build. Run both.
4. `W329_WORKLOAD` names a **runner**, not a hook — 8 arms, `rc=0`, zero probe logs, and my
   wrapper echoed "done" for each. ★ `w329_arm.sh` reported `⊘UNMEASURED` for every metric; the
   honest-absence design caught it in 80 seconds.

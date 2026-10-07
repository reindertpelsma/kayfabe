# single-store increment 4 — the boot evidence

**Measured 2026-09-14**, vast instance `51047925` (RTX 3060 / GA106, driver **580.159.04**,
49 cores), binary **`kayfabe-rev:eb9c1e800f392d8412809a17ab9093b8561d618e`** — `BINARY_REV ==
TREE_REV` on the graded boots.

⚠ **The revision is part of the citation.** Every row below is at that commit.

## The gate

| tag | `KAYFABE_SCRATCHPAD_CUDA` | `CUDA_WALK` | raw client |
|---|---|---|---|
| `e4ctl` | `off` | **`DISARMED`** | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |
| `e4cuda4` | `on` | **`OK`** | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |

```
SCRATCHPAD-CUDA AT REALIZE: CUDA_WALK=OK cuda_up=true device="NVIDIA GeForce RTX 3060"
  ptx_bytes=135668 bring_up_ms=2218.243 jit_ms=266.796 report_valid=true mapping_matched=true
  abi_refusal_fired=true
  report="magic=0x5257464b gen=1 pdbs=1 runs=1 entries=1796 refusals=0 refuse_mask=0x0
          sparse=1 trunc=false"
  probe_relaunch="PASS runs=1 (the mapping was re-reported)"
  probe_failed_launch="PASS refused as expected (cuLaunchKernel(deliberately malformed)
          refused: 1 (CUDA_ERROR_INVALID_VALUE)); and the context SURVIVED it — a following
          refresh returned runs=1 refusals=0"
```

⇒ inside a kayfabe boot, the VM-lifetime scratchpad isolate loaded the **committed PTX**,
JITted it, launched the walk kernel against a **synthetic VER2 page-table image it built
itself**, and got back a report that **validated** and carried **exactly the mapping the
fixture declared**. `refusals=0`, `refuse_mask=0x0`, `sparse=1` (the one declared-empty slot
the fixture plants so that counter is asserted rather than unexercised), `trunc=false`.

## §w724d's two probes, which the warm-up cannot stand in for

Both run **after** the mount namespace, the `pivot_root` and the privilege drop.

- **(a) can it launch again** — `PASS runs=1`. A full round trip: allocate, upload, three
  kernel launches, copy back, validate.
- **(b) can it survive a deliberately failed launch** — `PASS`. The driver refused a
  2048-thread block with `CUDA_ERROR_INVALID_VALUE`, **and the context still worked
  afterwards** (`runs=1 refusals=0`). ⊘ The second half is the half that matters: a driver
  that had to reopen something by path would fail *there*, not at the refusal.

## ⊘⊘⊘ THE FINDING §w724d DID NOT ANTICIPATE

`cuInit` refuses with **`CUDA_ERROR_OPERATING_SYSTEM` (304)** inside the isolate — and the
ordering §w724d prescribes cannot fix it, because an isolate is **born namespaced**.
`cuinit_namespace_bisection.txt`, measured on this box:

```
--user --map-root-user     cuInit=0   devices=1
--pid --fork               cuInit=304 devices=-1     ⇐
--mount                    cuInit=0   devices=1
--net                      cuInit=0   devices=1
--ipc                      cuInit=0   devices=1
--uts                      cuInit=0   devices=1
ALL SIX (the isolate)      cuInit=304 devices=-1
pid+mount, /proc REMOUNTED cuInit=0   devices=1      ⇐ the fix
user+pid+mount,/proc remntd cuInit=0  devices=1
```

⇒ **It is not the PID namespace. It is a PID namespace whose `/proc` is still the parent's.**
The fix is a mount (`kayfabe_linux_raw::sandbox::remount_proc`), not a weakened boundary —
and it is transient, because `sandbox::enter` puts a tmpfs over `/proc` moments later.

## The cost, on the VM-start path

| | ms |
|---|---|
| CUDA bring-up, `cuInit` → context → module → allocations | **2218** |
| of which `cuModuleLoadData` — **the PTX JIT** | **267** |
| (increment 1's isolate spawn, for comparison) | 1754 |

⇒ arming CUDA adds ~2.2 s to device realize. ⊘ The guest does not exist yet, so it pays none
of it — the same argument increment 1 made for the spawn. ⚠ It is **not** free: it is 2.2 s of
VM start-up time, and it should be read beside the spawn's 1.75 s rather than instead of it.

## ⚠ What these boots do NOT show

1. **The walker is not wired into refresh.** That is increment 6. This proves the kernel *can*
   run in-process and answer correctly, against an image whose answer was known beforehand —
   deliberately, so a red distinguishes "CUDA is broken in the isolate" from "the guest had
   not built its tables yet".
2. **The per-thread privilege drop is UNMEASURED.** `capset` is per-thread on Linux and CUDA's
   driver threads exist by the time `sandbox::enter` runs; `surrender_privilege`'s read-back
   reads `/proc/self/status`, which reports the calling thread only. Reaching the others needs
   a dirfd opened before the sandbox. See `THE_CONSTRAINTS.md` §w724d's correction block.
3. **One host, one driver, one chip.** VER3 (Hopper/Blackwell) is refused by name and has
   never decoded a table.

## The three earlier boots, kept

`cuda_census_lines.txt` carries all four armed attempts, because the failures are the
measurement:

| tag | `CUDA_WALK` | what it established |
|---|---|---|
| `e4cuda` | `NO_CUDA` | the glibc image exec'd and `dlopen` SUCCEEDED; `cuInit` refused 304 |
| `e4cuda2` | `REPORT_MALFORMED` | `/proc` remount fixed `cuInit`; the kernel ran and returned `runs=1 entries=1796`; our own validator refused it — `KFWR_MAGIC` was transcribed byte-reversed |
| `e4cuda3` | `REPORT_MALFORMED` | ⊘ the SAME failure after the fix: the QEMU binary was at HEAD and the **isolate image embedded inside it** was not (`kayfabe-cuda` was missing from `build.rs`'s rerun list) |
| `e4cuda4` | **`OK`** | the gate |

★ `e4cuda3` is the one worth keeping: **a stamp on the outer artifact says nothing about an
artifact embedded inside it**, and every "is the binary the tree" check this campaign owns was
looking at the outer one.

## Files

- `boot_e4cuda4.log` / `boot_e4ctl.log` — the harness's own output with its pre-registered
  outcomes; `boot_e4cuda.log` is the first armed attempt.
- `cuda_census_lines.txt` — every `SCRATCHPAD-CUDA` line from all four armed boots.
- `cuinit_namespace_bisection.txt` — the namespace bisection, verbatim.
- `run_e4*_dmesg.log` — the guest driver's own ring buffer.

⊘ The QEMU logs are ~7 MB each and are not committed; the lines that carry the result are
extracted above.

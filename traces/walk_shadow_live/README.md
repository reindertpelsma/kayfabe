# §6 step 1 — the LIVE walk shadow, GREEN

**Measured 2026-09-15**, vast instance `51067717` (RTX 3060 / **GA106**, driver
**580.159.04**, 15 cores), branch `w731-live-shadow`, binary
**`kayfabe-rev:5ebc5ae3…`** — `BINARY_REV == TREE_REV` on the graded boot.

⚠ **The revision is part of the citation.** Every row below is at that commit.

## ★★★★★ THE GATE

```
WALK-SHADOW compared=65 kernel_unavailable=2072 skipped[budget_spent=2072]
  host_runs=171 kernel_runs=171 host_unclassed=0 compared_flags=0xf
  image[pages_max=22 staged_bytes=3129344 absent_edges=0 sysmem_edges=0]
  disagreements=0 by_kind[none] first[]
  ⇒ ★★★ AGREEMENT over every compared refresh, on the fields both walkers decode.

[client] W392D_GUEST_OUTCOME=(P)  THREADS 8 of 8 verified ✔  MEAN_FALSIFIER=PASS
```

- **Not vacuous**: 65 comparisons, 171 runs on each side.
- **Clean**: zero disagreements, of any kind.
- **The raw client still passes**, and it also passed on the **control arm** at the SAME
  binary (`tag=w731ctl`, `KAYFABE_WALK_SHADOW=off`, `TREE_REV=87441e73`) — so the (P) is not
  a fact about a lucky boot, and the shadow is the only difference between the two.
  The control's own census line is the disarmed one, verbatim:
  `WALK-SHADOW ⊘ DISARMED — KAYFABE_WALK_SHADOW=off … This is NOT agreement and it is NOT a
  clean census; it is the absence of a measurement.`
- **`absent_edges=0`**: the kernel was given the complete tree, not a clipped one.

⊘ **What the zero does NOT say**, stated in the line itself: `compared_flags=0xf` is aperture
plus read-only. **Volatile, privilege, atomic-disable and `KIND` are decoded by the KERNEL
ONLY** and are not in this number. And the kernel walks a **relocated copy** holding exactly
the pages the host walk visited, so `missing_in_kernel` is fully live and `extra_in_kernel` is
live only within those pages.

⊘ **One host `Xid 31` fired on BOTH arms** — `CE0 HUBCLIENT_CE1 faulted @ 0xa0_00000000,
FAULT_PDE`, the standing w555 fault. `xid_both_arms.txt`. **Not caused by the shadow**, and
the control is what says so.

## ⊘ Three defects the boots found, in order — every one of them in the SHADOW

| boot | census | cause |
|---|---|---|
| v1 | `compared=0 skipped[isolate_refused=2115]` | **`CUDA_ERROR_INVALID_CONTEXT` (201).** A CUDA context is **current per thread**; the isolate brings CUDA up on its startup thread and serves requests on a worker. ⚠ `CUDA_WALK=OK` and both §w724d probes `PASS` on that very boot — they run on the bring-up thread. ⇒ w731m + the probe that would have caught it (w731n). |
| v2–v4 | `compared=65 disagreements=100 by_kind[extra_in_kernel=35 len_differs=35 missing_in_kernel=30]` | **Coalescing on unmasked flags.** The kernel cut one 4 GiB mapping in two at a `KIND` boundary — identical in every flag either side may compare. `walkdiff::canonical` coalesces on *equal flags*, so canonicalising raw runs preserved a boundary outside `COMPARED_FLAGS`. ⇒ **mask, then coalesce** (w731q). |
| v5 | `compared=65 disagreements=30 by_kind[missing_in_kernel=30]`, `absent_edges=53` | **A table is not a page.** A VER2 big-page table is **32 entries = 256 bytes**, named at 256-byte granularity; `PD3` is 32 bytes. Keying the image by `phys & !0xfff` threw the offset away, the edge went to the absent slot, and the kernel walked a zeroed table. ⇒ w731r. |

★ All three were found by the census printing **both sides of every disagreement verbatim**
rather than a count, plus a one-shot dump of the first disagreeing comparison's whole input.
The shadow's first findings were about the shadow — which is the instrument working.

## Files

- `boot_w731on.log` — the graded armed run, verbatim.
- `boot_w731ctl_control.log` — the control arm at the same binary.
- `census_lines.txt` — the census, the arena line and the CUDA census, verbatim.
- `first_disagreement_dumps.txt` — the v4 and v5 dumps that localised defects 2 and 3.
- `xid_both_arms.txt` — the standing `Xid 31`, on both arms.
- `boot_w731on_v1_invalid_context.log`, `boot_w731on_v2_100_disagreements.log` — the failing
  boots, kept because a census that was once wrong is the only thing that shows the fix
  moved it.

## How to reproduce

```
KAYFABE_SHIM_FEATURES="host-isolates cuda-scratchpad" \
  bash scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
PREFIX=w731off SHADOW=off bash scripts/bench/single_store_e6_boot.sh   # the control
PREFIX=w731on  SHADOW=on  bash scripts/bench/single_store_e6_boot.sh   # the armed arm
```
or `bash scripts/bench/w731_live_shadow_run.sh` on the bench box, which does both.

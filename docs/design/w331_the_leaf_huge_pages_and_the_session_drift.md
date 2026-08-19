# w331 — huge-page the leaf memfd: a NULL on the data plane, and a 34× session drift nobody was looking for

**STATUS: LIVE, 2026-08-19.** Written at master `844bafd5`, bench `48097794` (real GA106,
`580.159.04`). Two results below are measured and closed; §4's cause is **in flight** and is
labelled so.

⚠ **Read §3 before quoting any timing number measured on this bench today, including numbers
from `w330`.** Between-boot drift is larger than every effect any arm in this campaign has
claimed on `submit_ms`.

---

## 1. What was asked for, and where it came from

`w322_locate_the_operands.md` §6.8 ranked **"huge-page-back the leaf `memfd`"** as the cheapest
available performance fix: *"~4.9×, one allocation site, no address moves"*, gated on a
`pde_info` read. Our FB leaves are host sysmem described to RM as `PHYSICALITY_NONCONTIGUOUS`
4 KiB pages, and the guest's operands run at 2.51 GB/s against a link-saturating 12.33 on the
same GPU.

## 2. ⊘ BOTH PRESCRIBED MECHANISMS ARE UNAVAILABLE AS WRITTEN — measured

`[measured 2026-08-19, bench 48097794]`, 64 MiB `memfd` + `mmap(MAP_SHARED)` +
`madvise(MADV_HUGEPAGE)` + write-fault, reading `/proc/self/smaps` back:

| `shmem_enabled` | `madvise` rc | `ShmemPmdMapped` |
|---|---|---|
| `never` (**the shipped default**) | **0** | **0 kB** |
| `advise` | 0 | **65536 kB** — all of it |
| `never` again | 0 | **0 kB** |

⇒ ⊘⊘ **`madvise` SUCCEEDS AND DELIVERS NOTHING.** A `memfd` is **shmem**, and shmem THP is a
*separate* knob from `transparent_hugepage/enabled` (which read `[madvise]`, i.e. on, throughout).
The doc's second option is worse, not better: `HugePages_Total: 0` on this host, so `MFD_HUGETLB`
fails outright.

★ **The route is `shmem_enabled=advise`** — one sysfs write, no reserved pool, reversible. But it
is a **deployment requirement** whose failure mode is **silent 4 KiB**.

★ And a size floor, measured on the same host with the knob at `advise`:

| len | `pmd_backed` | forced 2 MiB-aligned base |
|---|---|---|
| 64 KiB | 0 (0 %) | 0 (0 %) |
| 512 KiB | 0 (0 %) | 0 (0 %) |
| 2 MiB | **100 %** | 100 % |
| 4 / 16 / 64 MiB | **100 %** | 100 % |

⇒ the kernel already 2 MiB-aligns every shmem mapping ≥ 2 MiB; forcing alignment changes nothing
in either direction. **A leaf under 2 MiB is out of scope, and that is a property of the leaf.**

## 3. ⊘⊘⊘ THE INSTRUMENT WAS BLIND, AND I HAD DOCUMENTED THE BLINDNESS AS DELIBERATE

The first boot of the change reported `pmd_backed=0` on **107 of 107 leaves** (88 × 2 MiB,
1 × 512 KiB, 18 × 64 KiB) with the knob at `advise`. That reads as *"the kernel refused"*.

**It is not a measurement.** The isolate is `pivot_root`ed into a sandbox with its own mount
namespace and **no `/proc`**, so `read_to_string("/proc/self/smaps")` fails — and the first
version of `pmd_mapped_bytes` returned **`0`** in that case, with a doc comment arguing the
collapse was fine because *"the caller's decision is identical in both cases"*. The known-positive
in §2 shows a 2 MiB shmem mapping gets **100 %** on this host, so the boot's `0` could not have
meant what it looked like.

Fixed at `844bafd5`: `Option<u64>` (`Some(0)` is a measurement, `None` is the absence of one),
plus `base_2m_aligned` and `len`, both derived from the pointer so they survive where `smaps`
does not.

⇒ Same class as `a_census_zero_needs_a_known_positive` — **in code written the same hour
specifically to avoid it.**

## 4. THE OUTCOME: a NULL on the data plane, and a drift that swamps the arm

Five arms, two builds, interleaved, `KAYFABE_BENCH_BW=28,31`, means over 7 iterations at 31 MiB:

| arm | build | knob | `submit_ms` | `sync_ms` |
|---|---|---|---|---|
| H1a | w331 (huge) | advise | 40.9 | **10.3** |
| H0a | master | advise | **6.3** | **10.3** |
| H1b | w331 (huge) | advise | 23.5 | **10.4** |
| H0b | master | advise | **122–141** | **10.2** |
| K | w331 (huge) | never | 157–216 | **10.3** |

★★★ **`sync_ms` is FLAT at 10.2–10.4 ms in all five arms.** That is the bandwidth-bound half
(w320: `cuCtxSynchronize` *is* the kernel running). ⇒ **the change moved the data plane by
nothing.** Corroborated independently: global `ShmemHugePages` peaked at **1 396 736 kB on H1b and
1 396 736 kB on H0b — identical to the kilobyte** — i.e. all of it is QEMU's 2 GiB guest RAM (which
QEMU madvises itself), and the ~177 MiB of leaves contributed **nothing measurable**. So the leaves
very likely never became huge at all, and §3 explains why we could not see that directly.

⊘⊘ **AND THE ARM IS UNMEASURED ON `submit_ms`, because the same commit gives 6.3 and 122–141.**
Within a boot the 7 iterations hold to ±2 %; the 34× is **between** boots and rises roughly with
position in the session. **Interleaving did not save it** — the drift is monotone, not alternating,
so alternating arms samples it at different points of a ramp rather than cancelling it.

⚠ **Consequence for everything else in this tree:** any `submit_ms`-like number compared across
boots on one host session is suspect unless the drift is controlled for. `w330`'s perf table
(gate 8.5×/19.4×, coalescer 12.7× on max) was measured that way.

**Cause: IN FLIGHT.** A three-boot run at one build and one knob is measuring whether boot
*position* alone reproduces it, with `/proc/buddyinfo` and free memory between boots. In-process
state cannot carry over — every boot is a fresh QEMU — so the candidates are host kernel memory
fragmentation and leaked host-RM objects / GPU VA from isolates that did not fully tear down. The
second would fit the shape exactly: our submit path does host RM work per launch, so an RM carrying
dead objects slows precisely that and leaves kernel execution untouched.

## 5. What ships

`request_huge_pages()` is **unconditional** at the FB-leaf mint, immediately before
`alloc_os_descriptor` (the point where RM fixes the GPU page geometry). Deliberately not an env
flag: the isolate is spawned with a **cleared environment** (`envp = {NULL}`) and that is a
security property, not an oversight — an env arm would open a config channel into it. The arm
selector for measurement is a second build.

The fault-in is a **read-modify-write of one byte per 4 KiB**, not a zero-fill: shmem THP is decided
at *fault* time and a read leaves the hole on the shared zero page, but a zero-fill would silently
destroy any leaf mapped after its first use. `requesting_huge_pages_preserves_every_byte_it_faults_in`
seeds a distinct value per page and asserts byte equality.
⊘ That test does **not** assert the return value: whether huge pages are granted is a property of
the host it runs on, so `>0` reddens a correct build on a stock host and `==0` reddens it on the
hosts we want.

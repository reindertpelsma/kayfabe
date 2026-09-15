# `walk_swap_live` — §6 STEP 2's BOOT: the walk kernel on the PUBLISH path

`[measured 2026-09-15, w732, vast 51076219, RTX 3060 GA106, host driver 580.159.04]`
Tree `93f6dc75` (`w732-walk-swap`); the binary's own stamp matched it on both arms
(`BINARY_REV=93f6dc75… TREE_REV=93f6dc75…`).

Two boots of `scripts/bench/w732_swap_run.sh`, differing in **one** variable:

| | `KAYFABE_WALK_SHADOW=on` (control) | `KAYFABE_WALK_SHADOW=swap` |
|---|---|---|
| raw client | **(P)**, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS` | **(P)**, `THREADS 8 of 8`, `MEAN_FALSIFIER=PASS` |
| `compared` | 65 | 65 |
| `disagreements` | **0** | **0** |
| `host_runs` / `kernel_runs` | 2930 / 2930 | 2930 / 2930 |
| `swap_armed` | false | **true** |
| `decided` | 0 | **65** |
| `fell_back` | none | **none** |
| verdict | `★★★ AGREEMENT` + `⊘ SWAP DISARMED` | `★★★ AGREEMENT` + `★★★ SWAP LIVE` |
| `image[pages_max]` | 22 | 22 |
| `absent_edges` / `sysmem_edges` | 0 / 0 | 0 / 0 |
| `inline_exceptions` | 0 | 0 |
| `worst_trap` | 16754 µs | 16536 µs |
| host `Xid` | 1 | 1 |

⇒ **Every one of the 65 comparisons became a decision**, `shared_root` never fired, and nothing
fell back. The guest paid nothing measurable: the worst trap moved the *wrong* way for a cost
story (down), `inline_exceptions` stayed 0, and the single host `Xid` appears on **both** arms —
so it is not the swap's.

## ⊘⊘⊘ WHAT THIS BOOT DOES **NOT** ESTABLISH — read before citing it

**The swap is observationally neutral by construction.** It substitutes only where the two
walkers agree, and under agreement the two leaf sets are the same set — the substitution asserts
that (`SwapRefusal::TargetChanged`) rather than hoping for it. ⇒ **no boot can distinguish a live
swap from a decider nobody consulted**, and neither the `(P)` nor any parity number is evidence
about the substitution. What this boot establishes is exactly three things:

1. the swap arm **boots and the raw client still passes** — putting the kernel on the publish
   path breaks nothing;
2. `decided=65 > 0` — the kernel **was** consulted, which is the only number that separates a
   live swap from a dead one;
3. `fell_back[none]` — no refusal, and each refusal would have printed at the moment it happened.

That the substitution reaches `AddressTable` **at all** is proved offline and deliberately so,
by a decider that disagrees on purpose: `tests/tests/walk_swap_decides.rs`.

⊘ And `★★★ AGREEMENT` is **not** the swap's verdict. The census prints the two side by side
precisely because an agreeing census with `decided=0` reads as a proven swap and is a census of
the old path — which is what the control arm's line literally is.

## ★★★ §3's FIRST PRECONDITION, MEASURED HERE AND NOT ASSUMED

Both arms: `E3-SPAN-PAGES=990381 E3-RESERVED_MB=4096 E3-ADVERTISED_MB=4096` ⇒
`E3-IDENTITY-WINDOW=FITS top=3868.7 MiB reserved=4096 MiB headroom=227.3 MiB`.

Every framebuffer page the guest named lies **inside the reservation**. ⊘ This corrects how
`[w730]`'s `span_pages=3087533` (= 12062 MiB, *"the tables sit ~11.8 GiB up a 12 GiB board"*) has
been read: that is a fact about the **ADVERTISED** size, not the reservation. `SCRATCHPAD FB-SIZE
derived_from_reservation=` rebinds the advertised size to what was actually reserved, and the
guest's tables move **down** with it — here to 3868.7 MiB of a 4096 MiB advertisement.

⇒ §3's identity window is **arithmetically possible**, and the invariant it depends on is now
stated where a boot can catch it: **advertise more than was reserved and it becomes impossible.**
⚠ One workload is not every workload; the guest used **94.5 %** of what it was told it had, so
the headroom is thin by nature rather than by luck. And this says nothing at all about the
promote/demote **fake range**, which is specified in `gpga_is_one_reserved_object.md` and built
by nobody.

## ⚠ ONE INSTRUMENT DEFECT, CAUGHT AND FIXED — the third of its class in two sessions

The run printed `E6-FALLBACK-LINES=1` beside `fell_back[none]`. The "fall-back" was the
**REALIZE banner's own prose**: *"…and prints a WALK-SWAP FALLBACK line of its own."* A counter
read a sentence **about** the record as an instance of it, and produced a flat contradiction a
reader could resolve either way. Fixed by anchoring on the record's shape (`WALK-SWAP FALLBACK
pdb=`) rather than its name.

⊘ Class: w731's `by_kind[` grep matching a different subsystem's census; w732's `AGREEMENT`
substring appearing inside a VACUOUS line (caught by an existing test, not by a boot).
**A log string is an interface the moment something greps it — and prose that quotes the
interface joins it.**

## Files

- `census_on.txt` / `census_swap.txt` — the `WALK-SHADOW` line, verbatim, one per arm.
- `realize_on.txt` / `realize_swap.txt` — the realize banners, the reservation, `CUDA_WALK`, the
  arena census.
- `run_summary.txt` — both arms' full graded summaries as the harness printed them.

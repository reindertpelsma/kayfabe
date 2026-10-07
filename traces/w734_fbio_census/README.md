# w734 — the two terms of §3's cost, measured, and what four boots established

`[measured 2026-09-15, vast 51082161, RTX 3060 GA106, driver 580.159.04]`
Tree revisions: `56edd0ed` (boots b, t, bar1) and `b203fea0` (boot c).

⊘ **Vast is compute, never storage.** These are the durable artefact; the box is gone.

| file | boot | what it is for |
|---|---|---|
| `run_w734b_summary.log` | `w734b` | the **primary** measurement — both terms, raw client `(P)` |
| `run_w734b_census_lines.log` | `w734b` | the census lines it was read from, verbatim |
| `run_w734b_host_xid.log` | `w734b` | the **pre-existing** `Xid 31 … CE0 … FAULT_PDE @ 0xa0_00000000` (w555/w711) — neither caused nor fixed here |
| `run_w734t_suite_census_lines.log` | `w734t` | the guest-suite attempt. ⊘ **A NARROWER workload, not a wider one** |
| `run_w734bar1_the_defect_the_client_missed.log` | `w734bar1` | ★ the two contradictory lines in one log, with the client still `(P)` |
| `run_w734c_bar1_128_knob.log` | `w734c` | the fix verified: `BAR1-AGREE ✔`, `BAR1-BUDGET ✔ FITS`, client `(P)` |

## ★★★ The headline: the ruling that ordered the branch was a derivation

`SINGLE_STORE_PLAN.md`'s *"§6 MUST PRECEDE §3"* and `THE_CONSTRAINTS.md` §w724c's *"there is
no working intermediate — **it does not boot**"* are one sum, **store bytes ÷ 48 MiB/s**, and
**neither term had ever been measured** — the tree contained no byte counter for store I/O at
all.

| term | the ruling | measured |
|---|---|---|
| read rate, device view of the reserved object | 48 MiB/s (uncited) | **52.5 – 55.8 MiB/s** ★ right |
| **write** rate | assumed the same | **4987 – 5012 MiB/s**, 95× — nobody had it |
| walk traffic per boot | 8.6 GiB (`7.3 MiB × 1178`) | **263 – 276 MiB** |
| ⇒ cost | *"~3 min … it does not boot"* | **5 – 10 s** |
| aperture (never named) | — | **128 – 131 frames ⇒ 0.5 MiB of 256**, `arm_us≈250`, `rel_us≈430` |

⊘ 5–10 s rather than a tidy number: the byte model **understates** `walk-bar`, whose ~3.5 M
reads average ~41 bytes and are **latency**-bound, not bandwidth-bound.

## ★ What replicated across boots (and what did not)

**Replicated, four boots:** `trap[frames=0]` (constraint 1 holds by measurement, not by
assumption); `WALK_FRAMES` 107–131; `cpu-ce` writes ~10–16 MiB; the device-view rate to within
6 %; and `IDENTITY-REACHED 3868.7 MiB = 94.5 %` **byte for byte identical** on every boot that
reached it.

⊘ **Not established: a wider workload.** `w734t` ran the 30-arm suite and `--concurrency` /
`--engines` failed and wedged the device, cascade-skipping 26 arms — the **pre-existing**
guest-suite state the plan already records. Its census is therefore a *narrower* workload
(146 MiB) and must not be read as the suite's number. ⚠ The *"one workload"* caveat stands.

## ⊘ The defect `w734bar1` found, and a green client did not

`ga106_profile` rebuilds from the **static** `GA106`, discarding the BAR1 patch. That boot
registered a **128 MiB** BAR1 and told the guest **256 MiB** — what `nvkvm.c:3552-3559` exists
to refuse, through a door that check cannot see (the two numbers *it* compares were both 128).
The raw client graded `(P)` with `THREADS 8 of 8` anyway.

⇒ `BAR1-AGREE` now compares what the device REGISTERS against what its emulated GSP TELLS THE
GUEST, every boot, and refuses at realize. `w734c` shows it passing — and with it, **§22 item
3's relation satisfied on this board for the first time**: `128 + 16 ≤ 256`, ✔ FITS, where every
previous boot printed ⊘⊘ DOES NOT FIT.

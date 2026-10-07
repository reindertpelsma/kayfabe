# `traces/w329_release/` — wiring the release of a joined framebuffer leaf

**STATUS: LIVE, 2026-08-15.** Every artefact here is from `vh2` (real GA106, RTX 3060, host
driver `580.159.04`, stock guest driver), branch `w329-wire-the-release` off master
`d859beb1`.

⊘ **The excerpt files are excerpts.** The whole QEMU log of a `28,31` boot is hundreds of MB;
what is committed is the lines the rung is graded on, and each file says which `grep` produced
it. A count taken from an excerpt is a count over the excerpt.

| file | what it is |
|---|---|
| `w329_all.log` | the launcher's own transcript — arm order, timestamps, terminator |
| `w329_arm_*.log` | one file per arm: every boot's `JOINREL`, `JOINTRAJ`, BW ladder and anchors |
| `w329_offline.log` | the offline suite, and ★ the master-baseline correction it forced |

★ **Read the trajectory, not the verdict.** `JOINTRAJ ... falls=N` is the pre-registered
falsifier: `w327` measured `joined=` climbing `0 → 83` over nine allocate/free cycles and
**never once falling**. A green `28,31` with `falls=0` would mean the failure was masked.

## The arms, and which log holds which

| prefix | list / workload | `KAYFABE_JOIN_RELEASE` | headline |
|---|---|---|---|
| `w329a1..3` | `28,31` | on, **before** the re-map guard | FAIL 3/3; `revoked=8` — ⊘ all eight were re-maps |
| `w329c1..3` | `28,31` | off (w327's state) | FAIL 3/3, `already joined=32` |
| `w329b1` | `4,64` | on, **before** the guard | ⊘ **REGRESSION** — `revoked=4`, PASS → FAIL |
| `w329bc1..2` | `4,64` | off | PASS 2/2 ⇒ the regression was OURS |
| `w329b2 1..2` | `4,64` | on, guard | PASS 2/2, `remaps_refused=4` |
| `w329a2 1..2` | `28,31` | on, guard | FAIL 2/2, `revoked=0 remaps_refused=8` — inert |
| ★ `w329sup1..3` | `28,31` | **supersede** | **PASS 3/3**, `already joined 32 → 0`, `SUPERSEDED=22` |
| ★ `w329sup64` | `4,64` | supersede | PASS, `SUPERSEDED=18` |
| `w329n1` | `4,16` NOLAUNCH | on | ★ known-positive `BENCH_NOLAUNCH_TOTAL_BAD=3670016` |
| `w329g/e/s/x` | cup3 / cup8 / N=3072 / R33 | on | all green |
| `w329sg/se/ss/sx` | cup3 / cup8 / N=3072 / R33 | **supersede** | all green — the arm the result rests on |

⚠ **`remaps_refused=` in these excerpts carries seventeen spaces before its value.** The
counter was emitted through a Rust string whose line-continuation backslash a Python heredoc
had eaten; it is fixed in source, and these logs predate the fix. Grep it as
`remaps_refused= *[0-9]*`.

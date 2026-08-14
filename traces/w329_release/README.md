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

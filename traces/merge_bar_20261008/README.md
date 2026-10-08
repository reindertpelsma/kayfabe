# Merge bar for the master candidate, 2026-10-08 (runs mcbox and mchost2)

**STATUS: RESEARCH, 2026-10-08 — evidence in progress; the app matrix and the 580 re-run of `0e64a960` are not finished.**

Two runs of `scripts/bench/box/merge_check.sh`, each on the exact revision named, each with its own log summary here.

| run | revision | box | GPU, driver | result |
|---|---|---|---|---|
| `mcbox` | `b5c9f717` | vast 54805775 (KVM template, ids are not secrets) | RTX 3060, 580.159.04 | [measured, mcbox at b5c9f717, 2026-10-08] tests 2396 passed 0 failed; v3 gates 9/9; kf3 built; bare-metal suite 30/30; fast guest suite 30/30; birth census OK, all channels PRIVILEGED_CHANNEL=0 |
| `mchost2` | `0e64a960` | trusted host, own hardware | RTX 4070, 595.91.07 | [measured, mchost2 at 0e64a960, 2026-10-08] tests 2396 passed 0 failed; v3 gates 9/9; bare-metal suite 30/30; fast guest suite 30/30; birth census OK |

What the two runs say about the raw client: [measured, mchost2 at 0e64a960, 2026-10-08] the same raw client that was refused at 595.91.07 before `claude/rawclient-all-drivers-20261008` (both suites 0/30, first merge-check attempt `mc20261009` at `b5c9f717`, 2026-10-08) now passes both suites there. [inferred] 580 still passes with the new client: `mcbox` ran the old client at `b5c9f717`; a 580 run of `0e64a960` is still to do.

The first attempt on the trusted host (`mc20261009` at `b5c9f717`, 2026-10-08) stopped at the bare-metal stage for that reason and is not part of the table.

CUDA ladder on the box, one rep, `b5c9f717` (`box_lanes_so_far.txt`): [measured, mcl_host and mcl_guest at b5c9f717, 2026-10-08] all four rungs (cup2, cup3, cup8, cup8bench) PASS in both host and guest; the guest 2048 x 2048 matmul runs at 784 GFLOPS against 786 on the host.

App matrix on the box (`b5c9f717`, run `mcapps`): [measured, mcapps at b5c9f717, 2026-10-08] bare-metal pass 71/71. The guest pass was still running when this file was committed.

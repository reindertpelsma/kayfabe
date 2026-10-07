# w326 — the publish trigger, measured

Boots on `vh` (RTX 3060 GA106, host driver 580.159.04, stock guest driver), 2026-08-14.
Source revision is stamped in each arm log's `REV=` line — the tree is rsync'd, not cloned,
so `git rev-parse` has no repo to answer from and a stamped file is the honest substitute.

| file | arm | what it is |
|---|---|---|
| `w326_arm_w326m.log` | measurement, tick **absent from the binary** | the decoder's first boot: `MMUINVAL` + `CUP3_VAL=43` |
| `w326_arm_w326r.log` | reclaim tick **ON**, `cup3`, **n=3** | the armed arm |
| `w326_arm_w326o.log` | reclaim tick **OFF**, `cup3`, n=1 | ★ the control, **same binary** as `r` — the only thing separating them is one word in the environment |
| `w326_arm_w326e.log` | reclaim tick ON, `cup8`, n=1 | the quietly-wrong oracle (`CUP8_BAD` / `CUP8_MAXERR`) |
| `w326_arm_w326x.log` | reclaim tick ON, `R33 arm 1`, n=1 | a different mapping path |
| `w326_all.log` | — | the single launcher, with each arm's completion timestamp |
| `mmuinval_census.txt` | — | the `MMUINVAL` line from every boot, side by side |

⊘ **Read the arm logs, not the qemu logs**: every metric is anchored and an absent one prints
`⊘UNMEASURED` rather than `0`. A missing `TERMINATOR` line means the job did not finish, which
is a different fact from a bad result.

# Ada and Blackwell at current master — merge bar + CUDA ladder, 2026-09-30

**STATUS: DATA.** Two dies not measured before, each on a rented Vast KVM-template box, host driver
re-provisioned to **580.159.04 open** (`provision_full.sh`), guest driver 580.159.04. With
`traces/v3_turing_master/` (TU116) this covers Turing, Ampere (the GA106 bench), Ada and Blackwell at
the current master code.

| die | box | merge bar (verify HEAD) | tests | gates | bare | thin | CUDA ladder (rev) |
|---|---|---|---|---|---|---|---|
| **AD104** — NVIDIA RTX 4000 Ada Generation, 20 GiB | vast 53568615 | `fc2dadc4` (code `f442980e`, incl. the RAM-object fix) | 1744 / 0 | 9/9 | 30/30 | 30/30 | host 4/4, guest 4/4 (`d40dafbf`) |
| **GB205** — GeForce RTX 5070, 12 GiB (`10de:2f04`) | vast 53567791 | `d40dafbf` (code `3f67ed95`) | 1742 / 0 | 9/9 | 30/30 | 30/30 | host 4/4, guest 4/4 (`d40dafbf`) |

Every bar: KF3_RC=0, FG_RC=0, `EXIT rc=0`. Ladder rungs: cup2 `CE rv=0xabcd1234`, cup3 `CUP3_VAL=43`,
cup8 `CUP8_BAD=0 CUP8_MAXERR=0`, cup8bench every timed iteration verified. Per-box logs:
`ada1.tgz`, `bw1.tgz` (merge log, gates, tests, thin-suite run, both ladder logs, chain log, provisioning
log). ⚠ The ladder ran from the provisioned checkout (`d40dafbf`), the merge bar from its own verify
worktree; both revisions are named above. Earlier family rows: AD106 and GB203/GB206 in
`design/V3_FAMILY_PORT_ADA.md`, `V3_FAMILY_PORT_BLACKWELL.md`, `V3_DOORBELL_IOEVENTFD.md` §7.6.

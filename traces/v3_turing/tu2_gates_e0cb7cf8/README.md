# Turing gates re-check — TU116 at `e0cb7cf8`

**STATUS: LIVE, 2026-09-28.** Box: vast `53080587`, GTX 1660 SUPER (TU116, `0x21C4`), host driver
580.159.04 open, nested KVM. Checkout `tuwork` = `e0cb7cf8` exactly (`HEAD=e0cb7cf8 dirty=0`, first
lines of `gates.log`), which contains `46a66099` (the gates' CE predicate derived from the class
tables instead of a hand list that started at Ampere).

| what ran | result | file |
|---|---|---|
| `bash scripts/bench/v3_gates.sh` | **9/9** (`V3_GATES_SUMMARY pass=9 fail=0`) — gate3 and gate4, which failed at `80e13bc5` on `ForeignClass { class: 50613 }` (`0xC5B5` TURING_DMA_COPY_A), now PASS | `gates.log`, `job.log` |

⚠ `walk_submit … budget=50 (MISSED)` is a measurement row, not a verdict — it reads MISSED on GA106 and
GB203 too (`V3_FAMILY_PORT_BLACKWELL.md` §6). TU116: p50 33 / 47 µs (ver2 / ver3).

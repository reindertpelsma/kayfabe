# Native allocation parameters for Windows ring migration

**STATUS: RESEARCH, 2026-10-12.** Prepared; no new hardware result yet.

H-ALLOC-OBS and its falsifier precede the run in the restricted design review.
The former reference saved only the fn103 allocation prefix. Public source shows
a32-byte allocation body prefix and full committed transport-element copies;
observer now captures only declared params within those validated stable pages.
Flag8 identifies params outside the RPC checksum; updated decoder keeps checksum
coverage separate from stable snapshots/export hash. Oversized claims keep only
prefix. Bounds/core GCC+Clang ASAN/UBSAN/decoder tests PASS; red/green logs here.

A fresh original-baseline child, original600s Gravity workload/pinned72741e2b
matrix oracle/driver580.88/RTX4070. Personal1380 untouched. New standalone QEMU
built from10.2.4 tar plus public observer; no refusal/driver-analysis patches.
Capture window300s from each boot, same fixed source/output bounds. A limit is
reported as missing coverage, never absence of migration. Input key events are
not traced. Do not archive guest password, RAM, disk or environment. No native
cursor rule or Windows product compatibility claim follows from alloc params.

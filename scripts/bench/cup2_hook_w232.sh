#!/usr/bin/env bash
# ★★★ POST_CAPTURE_HOOK for w232 — run `cup2` so a **USER proc** rings `Ce` doorbells.
#
# ⊘ WHY THIS EXISTS, and it is a measurement, not a convenience. `boot_capture.sh` alone
# loads the driver and runs `nvidia-smi`; `[measured 2026-08-11, boots `w232a`/`w232b`]`
# that produces **2 doorbells, both the SYSTEM proc's** (`proc=0 chan=1`), and a
# `RING-ROSTER` of 3 kernel clients with no user proc in it at all.
#
# ⇒ Without this hook, "the forwarding fall-through was not reached" is **unreadable**: it
# cannot be told from "no user proc ever rang a `Ce` doorbell", which is the trap
# `CLAUDE.md` records and this rung's brief warned about. With it, the same boot reports
# `191 arrived, 183 served, 8 REFUSED` — reproducing `w229`/`w230`'s population exactly —
# and 8 of those doorbells are `proc=2`'s.
#
# ⚠ `cup2` is NOT expected to pass; `CUP2_RC=124` (the 180 s timeout at `cuCtxCreate`) is
# the standing wall on both executor arms. What this hook is for is the **population**.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup2_hook_w232.sh scripts/bench/boot_capture.sh <tag>
#   ⚠ needs GQ_TIMEOUT >= 240 — the default 90 s is shorter than cup2's own 180 s deadline.
set -uo pipefail
# ★ The REPO copy, resolved from this script's own location — never a box-local
# `/workspace/bench/gssh_nv`. See `boot_capture.sh` phase 0 for the boot whose whole
# differential was taken by a `sed -i` on the box copy.
G="$(cd "$(dirname "$0")" && pwd)/gssh_nv"
$G "cat > /tmp/cup2.c" < /workspace/bench/cup2.c || { echo "PUSH_FAILED"; exit 2; }
$G "gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_RC=\$?"
$G "cd /tmp && timeout 180 ./cup2 2>&1 | tail -30; echo CUP2_RC=\${PIPESTATUS[0]}"

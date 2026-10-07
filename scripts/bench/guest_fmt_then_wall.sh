#!/usr/bin/env bash
# POST_CAPTURE_HOOK — run BOTH of §16.33's falsifiers in ONE boot.
#
# §16.33's root cause (our empty `0x00801813` reply zero-fills the guest's params, so
# `numEntries` reaches `gvaspaceExternalRootDirCommit` as 0 and `gpu_vaspace.c:3094` fires)
# has two independent tests, and they fail in different directions:
#
#   1. `guest_vaspace_fmt.sh` — the ROOT FORMAT, read from a stock driver via `0x801806`.
#      The arithmetic REQUIRES a correct GA10x root `[48:47]`, i.e. **`vaBitCount = 49`**.
#      57 or 1 refutes the arithmetic *even if the assert moves*.
#   2. `guest_cuinit_wall.sh` — the WALL, on both the RM and UVM planes. Three-valued:
#      `:3094` again = refuted; `:3094` gone = confirmed; a DIFFERENT `gpu_vaspace` row
#      (`:3097 :3109 :3115 :3126 :3133 :3142 :3149 :3176 :3200 :3206 :3240`) = confirmed AND
#      the wall moved inside `commit`; `:3332` alone = ambiguous, never scored on its own.
#
# ⊘ Running them in separate boots would cost two boots AND make them non-comparable: the
# emulated GSP's WPR2 only resets on a full QEMU restart, so two boots are two different
# machines. One boot, both readings, same clock.
#
# ★ ORDER MATTERS AND IS DELIBERATE: the format probe runs FIRST, because it allocates its
# own client/device/VAS and `guest_cuinit_wall.sh` clears the kernel ring buffer before it
# runs `cup2`. Reversed, the probe's own RM traffic would land inside the window that is
# supposed to be `cuInit`'s alone — the exact contamination `guest_cuinit_wall.sh` clears
# the ring to prevent.
#
# ⊘ Neither hook's exit status aborts the other: a failed instrument is a fact about that
# instrument, and losing the other reading with it would be the opposite of the point. Both
# statuses are printed and the caller judges them separately.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
TAG=${1:-fmtwall}

echo "############ HOOK 1/2 — the ROOT FORMAT (falsifier B) ############"
"$SELFDIR/guest_vaspace_fmt.sh" "$TAG"
echo "FMT_HOOK_RC=$?"

echo "############ HOOK 2/2 — the WALL (falsifier A) ############"
"$SELFDIR/guest_cuinit_wall.sh" "$TAG"
echo "WALL_HOOK_RC=$?"

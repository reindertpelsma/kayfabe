#!/usr/bin/env bash
# POST_CAPTURE_HOOK for scripts/bench/boot_capture.sh: kfml inside the booted, OFFLINE fat guest (design 2026-11-17, never run in a guest).
# boot_capture.sh calls it with the guest up and its driver loaded:   POST_CAPTURE_HOOK=scripts/ci/kfml_guest_hook.sh bash scripts/bench/boot_capture.sh <tag>
# kfml_stage.py --where guest probes the guest, builds the wheelhouse on the host, copies it over the guest transport (gssh_nv), installs it there
# offline with hashes, runs the tier and writes the result document; see scripts/ci/kfml_guest.py. Env: KF_KFML_SHA (required), KF_KFML_TIER, KF_KFML_PREFIX.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TAG="${1:?usage: kfml_guest_hook.sh <tag>}"
OUT="${KF_KFML_GUEST_OUT:-${BENCH_DIR:-/workspace/bench}/kfml_guest_${TAG}.json}"
rc=0
python3 "$HERE/kfml_stage.py" run --where guest --sha "${KF_KFML_SHA:?KF_KFML_SHA is required}" --checkout "$ROOT" \
  --prefix "${KF_KFML_PREFIX:-/workspace/kf-kfml-guest}" --out "$OUT" --tier "${KF_KFML_TIER:-mid}" --run-id "$TAG" \
  --guest-ssh "$ROOT/scripts/bench/gssh_nv" || rc=$?
echo "KFML_GUEST_OUT=$OUT"
echo "KFML_GUEST_RC=$rc"
exit "$rc"

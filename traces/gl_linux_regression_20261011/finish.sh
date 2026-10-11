#!/usr/bin/env bash
# Run under the GPU flock after the original 02afce90 runner exits. Product source stays fixed.
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=02afce90
OUT=$W/codex-gl-regression-20261011-$REV
cd /data/kf-gl730101-host
test "$(git rev-parse --short=8 HEAD)" = "$REV"
test -z "$(git status --porcelain --untracked-files=no)"
grep -q 'FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30' "$OUT/logs/fast.log"
echo "POSTCHECK_START rev=$REV $(date -Is)"
LINK=/workspace/bench/kf3-bins/$REV
CREATED_LINK=0
trap 'rc=$?; if [ "$CREATED_LINK" = 1 ]; then rm -f "$LINK"; fi; echo "POSTCHECK_EXIT rc=$rc $(date -Is)"' EXIT
cp "$OUT/logs/tspace.log" "$OUT/logs/tspace_initial.log"
logs=("$OUT"/bench/fast_codexglreview_*_qemu.log)
test "${#logs[@]}" = 30
{
    for log in "${logs[@]}"; do
        echo "LOG=$log"
        # The root raw client is ADMIN in guest RM, so --user's USER-only census does not apply.
        python3 -I scripts/p1p2/tspace_log_gate.py "$log"
    done
} > "$OUT/logs/tspace.log" 2>&1
echo "TSPACE_LOGS_PASS=30 $(date -Is)"
if [ ! -e "$LINK" ] && [ ! -L "$LINK" ]; then
    ln -s "$W/readtrace/kf3-bins/$REV" "$LINK"
    CREATED_LINK=1
fi
cmp "$LINK/qemu-system-x86_64" "$W/readtrace/kf3-bins/$REV/qemu-system-x86_64"
dmesg | grep -a Xid > "$OUT/logs/broker_xid_before.txt" || true
KF_LOCK=$OUT/private.lock KF3_REV=$REV bash scripts/bench/display/input_proof.sh codexglreview > "$OUT/logs/broker.log" 2>&1
dmesg | grep -a Xid > "$OUT/logs/broker_xid_after.txt" || true
cp "$W/broker-interactive/proof-codexglreview/proof.log" "$OUT/logs/broker_proof.log"
if grep -Eq '^PROOF_.*FAIL' "$OUT/logs/broker_proof.log"; then exit 1; fi
grep -q '^PROOF_DESKTOP cinnamon_up' "$OUT/logs/broker_proof.log"
grep -q '^PROOF_EXIT ' "$OUT/logs/broker_proof.log"
echo "POSTCHECK_DONE $(date -Is)"

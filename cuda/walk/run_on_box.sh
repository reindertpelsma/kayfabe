#!/bin/bash
# Drive the walk-kernel suite on a disposable CUDA container.
#
# Source only ever travels box-WARD, out of a commit (`git archive`). Nothing
# executable ever comes back: the box returns a log.
#
# ⚠ Traps this file exists to avoid:
#   - an empty log is NOT "still running": the remote writes a START marker and
#     an `EXIT=<n>` terminator, so "file exists but has no terminator" is a
#     detectable state of its own;
#   - 143 (the job was SIGTERM'd, dead) and 124 (the LAUNCHER timed out, the job
#     is fine) mean opposite things — we read the terminator, never the launcher;
#   - a kill goes on a line of its own, in its own ssh invocation, because a
#     later word on the same command line re-matches the pattern.
set -u
HOST="${HOST:-wk}"
REMOTE=/root/kfwalk
TAG="${1:-run}"
LOG="/tmp/kfwalk_${TAG}.log"

here=$(cd "$(dirname "$0")/../.." && pwd)

echo "== shipping source from $(git -C "$here" rev-parse --short HEAD) =="
git -C "$here" archive HEAD cuda/walk | ssh "$HOST" "rm -rf $REMOTE && mkdir -p $REMOTE && tar -x -C $REMOTE" || exit 1

ssh "$HOST" "pkill -f '[k]f_tests' >/dev/null 2>&1; true"

ssh "$HOST" "cd $REMOTE/cuda/walk && nohup sh -c '
  echo START \$(date -u +%FT%TZ) \$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader)
  make check-invariants; rc_inv=\$?
  make kf_tests 2>&1;   rc_build=\$?
  make check-ptx;       rc_ptx=\$?
  if [ \$rc_build -ne 0 ]; then echo BUILD_FAILED; echo \"EXIT=\$rc_build\"; exit 0; fi
  timeout 900 ./kf_tests; rc=\$?
  nvidia-smi --query-gpu=name --format=csv,noheader >/dev/null 2>&1 || echo GPU_UNRESPONSIVE
  dmesg 2>/dev/null | grep -i xid | tail -5
  echo \"INV=\$rc_inv BUILD=\$rc_build PTX=\$rc_ptx\"
  echo \"EXIT=\$rc\"
' > $REMOTE/out.log 2>&1 &" || exit 1

echo "== waiting for the terminator =="
for i in $(seq 1 180); do
    sleep 5
    if ssh "$HOST" "grep -q '^EXIT=' $REMOTE/out.log 2>/dev/null"; then
        break
    fi
    alive=$(ssh "$HOST" "pgrep -c -f '[k]f_tests|[m]ake' 2>/dev/null || echo 0")
    if [ "$i" -gt 3 ] && [ "$alive" = "0" ]; then
        echo "!! no terminator and nothing running -- the job died"
        break
    fi
done

ssh "$HOST" "cat $REMOTE/out.log" > "$LOG"
echo "== log: $LOG =="
cat "$LOG"
if grep -q '^EXIT=0$' "$LOG"; then
    echo "RESULT: pass"
    exit 0
fi
term=$(grep '^EXIT=' "$LOG" | tail -1)
if [ -z "$term" ]; then
    echo "RESULT: NO TERMINATOR -- the run did not finish (this is not 'still running')"
    exit 3
fi
echo "RESULT: fail ($term)"
exit 1

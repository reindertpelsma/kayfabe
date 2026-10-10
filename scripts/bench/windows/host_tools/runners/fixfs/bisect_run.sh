#!/usr/bin/env bash
# Run the --timer arm for each built revision (fast_suite/run_fast_guest take the GPU flock).
# Uses the validation agent's fast guest (initrd built at 6fafcc6e; the controls used the same).
set -uo pipefail
W=/var/lib/kf-windows-20261005
F=$W/fixfs
mkdir -p $F/bench
cd $W/kayfabe-val-6fafcc6e  # scripts at 6fafcc6e for every candidate, as the controls
echo "BISECT_RUN_START $(date -Is) revs=$*"
for rev in "$@"; do
    q=$F/kf3-bins/$rev/qemu-system-x86_64
    [ -x "$q" ] || q=$W/kf3-bins/$rev/qemu-system-x86_64
    [ -x "$q" ] || { echo "NO_BIN $rev"; continue; }
    KF_DEVICE=kf3 BENCH_DIR=$F/bench KF_FASTGUEST_DIR=$W/val-6fafcc6e/bench/fastguest QEMU_BIN=$q \
        timeout 600 bash scripts/fastguest/fast_suite.sh "b$rev" 180 --timer 2>&1 | grep -E "FAST_CELL_ARM|FAST_SUITE_PASS"
    ql=$F/bench/fast_b${rev}_timer_qemu.log
    echo "RESULT $rev dead=$(grep -c 'DEAD:' $ql 2>/dev/null) poisoned=$(grep -c 'POISONED' $ql 2>/dev/null) carve_refused=$(grep -c 'inside the firmware carve-out' $ql 2>/dev/null)"
done
echo "BISECT_RUN_DONE $(date -Is)"

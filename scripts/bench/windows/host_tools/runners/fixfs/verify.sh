#!/usr/bin/env bash
# Hardware verification of ONE revision: build_kf3 -> raw client -> fast guest -> fast suite (30 arms)
# -> tspace log gate on every arm -> v3_gates (under the GPU flock). Strictly serial.
set -uo pipefail
REV=${1:?rev}
W=/var/lib/kf-windows-20261005
F=$W/fixfs
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=$F/target
cd $F/repo
git checkout -q --detach "$REV" || { echo "CHECKOUT_FAIL"; exit 1; }
REV=$(git rev-parse --short=8 HEAD)
echo "VERIFY_START $REV $(date -Is) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
bash scripts/bench/build_kf3.sh $F/qemu-src $F/qemu-build > $F/v_${REV}_build_kf3.log 2>&1; echo "BUILD_KF3_EXIT $? $(date -Is)"
grep KF3_BUILT $F/v_${REV}_build_kf3.log
KAYFABE_BUILD_REV=$REV cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > $F/v_${REV}_client.log 2>&1; echo "BUILD_CLIENT_EXIT $? $(date -Is)"
mkdir -p $F/client-$REV && cp $F/target/release/kayfabe-rm-ladder $F/client-$REV/
KF_FROM_HOST=1 CLIENT=$F/client-$REV/kayfabe-rm-ladder bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 $F/fastguest-$REV > $F/v_${REV}_fastguest.log 2>&1; echo "FASTGUEST_EXIT $? $(date -Is)"
KF_DEVICE=kf3 BENCH_DIR=$F/bench KF_FASTGUEST_DIR=$F/fastguest-$REV QEMU_BIN=$F/kf3-bins/$REV/qemu-system-x86_64 \
    timeout 4000 bash scripts/fastguest/fast_suite.sh v$REV 180 > $F/v_${REV}_fast.stdout 2>&1; echo "FAST_EXIT $? $(date -Is)"
grep -E "FAST_SUITE_PASS" $F/v_${REV}_fast.stdout
for q in $F/bench/fast_v${REV}_*_qemu.log; do
    a=$(basename $q _qemu.log); a=${a#fast_v${REV}_}
    echo "ARM $a dead=$(grep -c 'DEAD:' $q) poisoned=$(grep -c 'POISONED' $q) carve_whole_refused=$(grep -c 'inside the firmware carve-out' $q) clipped_lines=$(grep -c 'CLIPPED at the firmware carve-out' $q)"
    python3 -I scripts/p1p2/tspace_log_gate.py $q --user > $F/v_${REV}_gate_$a.txt 2>&1; echo "GATE $a rc=$?"
done
exec 9>/tmp/kayfabe-fastguest.lock
flock 9
echo "GATES_LOCKED $(date -Is)"
bash scripts/bench/v3_gates.sh $F/v_${REV}_v3_gates.log > /dev/null 2>&1; echo "V3_GATES_EXIT $? $(date -Is)"
grep -E "V3_GATES_SUMMARY|V3_GATES_BIRTHS|HEAD=" $F/v_${REV}_v3_gates.log
flock -u 9
echo "VERIFY_DONE $REV $(date -Is)"

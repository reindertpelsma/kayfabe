#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ada2build_variants.log 2>&1
trap 'rc=$?; echo "VARIANTS_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "VARIANTS_START $(date -u -Is)"
cd /root/kayfabe
export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/root/kayfabe/target CARGO_BUILD_JOBS=4
test -z "$(pgrep -x qemu-system-x86 || true)"
for spec in \
 'caps00c codex/ad102-ablate-caps00c-20261016 a52494d487e2ff3068cbae60e3ff1fe422ff281d' \
 'lpwr2801 codex/ad102-ablate-lpwr2801-20261016 a7509bf3d33a6986874beae0d03c95e6ef0efe19' \
 'lpwr2806 codex/ad102-ablate-lpwr2806-20261016 003ad0e749eede980723cc64b9107a1d747b9cc7'; do
 read -r label branch source <<< "$spec"
 git fetch -q origin "$branch"
 git checkout --detach "$source"
 test "$(git diff --numstat 9d6f210ba -- crates/kf-disp/data/caps_measured.tsv crates/kf-rm/data/measured_constants.tsv | awk '{a+=$1;b+=$2}END{print a ":" b}')" = 0:1
 echo "ABLATION_FALSIFIER_BEFORE_RUN label=$label: removing only this row still passesCode0/desktop/CUDA/D3D; then not required for tested580/profile."
 timeout --signal=TERM 600 bash scripts/bench/v3_gates.sh "/root/prov/ada2build_${label}_gates.log" >"/root/prov/ada2build_${label}_gates_wrapper.log" 2>&1
 timeout --signal=TERM 600 bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 >"/root/prov/ada2build_${label}_build.log" 2>&1
 echo "VARIANT_READY label=$label source=$(git rev-parse HEAD) $(date -u -Is)"
done
git checkout --detach 9d6f210ba
for rev in 9d6f210b a52494d4 a7509bf3 003ad0e7; do
 mkdir -p "/root/ada2build-wrappers/$rev"
 cp /root/ada2build_qemu_wrapper.py "/root/ada2build-wrappers/$rev/qemu-system-x86_64"
 chmod 755 "/root/ada2build-wrappers/$rev/qemu-system-x86_64"
done
echo "VARIANTS_COMPLETE $(date -u -Is)"

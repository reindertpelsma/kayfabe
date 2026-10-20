#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/bwr_provision.log 2>&1
trap 'rc=$?; echo "BWR_PROVISION_EXIT rc=$rc $(date -u -Is)"' EXIT
. /root/bwr/common.sh
echo "BWR_PROVISION_START own_instance=56909569 $(date -u -Is)"
export KAYFABE_BRANCH=codex/sweep-integrate-20261018 CARGO_BUILD_JOBS=48
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
cd /root/kayfabe
git fetch --depth 1 origin $BASE
git checkout --detach $BASE
[ "$(git rev-parse HEAD)" = $BASE ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git fetch -q --deepen=512 origin codex/sweep-integrate-20261018 integration/prod-candidate-5-20261018
nohup bash /root/bwr/media.sh </dev/null >/root/prov/bwr_media_controller.log 2>&1 &
echo $! >/root/prov/bwr_media.pid
bash scripts/bench/provision_host_driver.sh >/root/prov/driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
bash scripts/bench/host_preflight.sh >/root/prov/bwr_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/bwr_host_preflight.log
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/bwr_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/bwr-ctl-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/bwr-ctl-probe >/root/prov/bwr_host_die.log 2>&1
cat /root/prov/bwr_host_die.log
grep -Eq 'PROBE_DIE .*die=GB202' /root/prov/bwr_host_die.log
bash scripts/bench/v3_gates.sh /root/prov/bwr_gates.log >/root/prov/bwr_gates.run 2>&1
grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' /root/prov/bwr_gates.log
bash scripts/bench/provision_bench_tree.sh >/root/prov/tree.log 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/bwr_raw_build.log 2>&1
export KF_DEVICE=kf3
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/bwr_fast595_build.log 2>&1
[ -x /workspace/bench/kf3-bins/7f22d18d/qemu-system-x86_64 ]
sha256sum /workspace/bench/kf3-bins/7f22d18d/qemu-system-x86_64 >/root/prov/bwr_qemu_sha256
echo "BWR_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
# Windows install is CPU-only (std VGA, no kf3 device): runs beside the native GPU baseline.
nohup bash /root/bwr/install.sh </dev/null >/dev/null 2>&1 & ipid=$!
bash /root/bwr/native_baseline.sh
echo "BWR_NATIVE_DONE $(date -u -Is)"
nohup bash /root/bwr/candidate_build.sh </dev/null >/dev/null 2>&1 & cpid=$!
wait $ipid || true
grep '^WIN_BASE_INSTALL_EXIT rc=0 ' /root/prov/bwr_install.log
bash /root/bwr/bind.sh
wait $cpid || true
grep '^CANDIDATE_SEPARATE_BINARY_READY' /root/prov/bwr_candidate_build.log
echo "BWR_READY_FOR_ARMS $(date -u -Is)"

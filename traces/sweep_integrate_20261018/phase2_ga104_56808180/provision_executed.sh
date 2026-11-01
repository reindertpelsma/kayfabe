#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/sweep_20261020_provision.log 2>&1
trap 'rc=$?; echo "SWEEP_PROVISION_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "SWEEP_PROVISION_START own_instance=56808180 $(date -u -Is)"
export KAYFABE_BRANCH=codex/sweep-integrate-20261018 CARGO_BUILD_JOBS=6
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8
git checkout --detach e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8
[ "$(git rev-parse HEAD)" = e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8 ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git fetch -q --deepen=512 origin codex/sweep-integrate-20261018 integration/prod-candidate-5-20261018
git rev-parse HEAD >/root/prov/sweep_product_source
bash scripts/bench/provision_host_driver.sh >/root/prov/driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
# Stock driver installation is a provisioning prerequisite; health must pass
# before any kayfabe gate, guest or timing is attributed to this box.
bash scripts/bench/host_preflight.sh >/root/prov/sweep_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/sweep_host_preflight.log
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/sweep_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/sweep-ctl-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/sweep-ctl-probe >/root/prov/sweep_host_die.log 2>&1
cat /root/prov/sweep_host_die.log
grep -Eq 'PROBE_DIE .*die=GA104' /root/prov/sweep_host_die.log
git fetch -q origin integration/prod-candidate-5-20261018
[ "$(git rev-parse FETCH_HEAD)" = f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e ] || { echo INTEGRATION_ADVANCED_STOP; exit 4; }
# Falsifier: a native gate or stock workload fails; do not attribute that to kayfabe.
bash scripts/bench/v3_gates.sh /root/prov/sweep_gates.log >/root/prov/sweep_gates.run 2>&1
grep -E '^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$' /root/prov/sweep_gates.log
bash scripts/bench/provision_bench_tree.sh >/root/prov/tree.log 2>&1
# Explicitly rebuild source-matched raw client and fast guest after the normal QEMU flow.
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/sweep_raw_build.log 2>&1
export KF_DEVICE=kf3
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/sweep_fast595_build.log 2>&1
[ "$(sed -n 's/^version=//p' /workspace/bench/fastguest595/driver.STAGED)" = 595.91.07 ]
# Pin every later workload to this per-revision binary; never pull during a timed run.
[ -x /workspace/bench/kf3-bins/e8553e6c/qemu-system-x86_64 ]
sha256sum /workspace/bench/kf3-bins/e8553e6c/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
# Isolate driver images before any fat guest workload can write the580base.
JOBS=8 bash scripts/drivermatrix/stage_guest_driver.sh 580.159.04 /workspace/drivers >/root/prov/sweep_module580_stage.log 2>&1
KF_FROM_HOST=1 KF_GUEST_DRIVER_DIR=/workspace/drivers/580.159.04 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest580 >/root/prov/sweep_fast580_build.log 2>&1
mkdir -p /workspace/drivers/595.91.07
ln -sfn /root/NVIDIA-Linux-x86_64-595.91.07.run /workspace/drivers/595.91.07/NVIDIA-Linux-x86_64-595.91.07.run
KF_STAGE_SMP=8 bash scripts/drivermatrix/stage_fat_guest.sh 595.91.07 /workspace/drivers >/root/prov/sweep_stage_fat595.log 2>&1
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
qemu-img convert -O qcow2 /workspace/bench/guest-595.91.07.qcow2 /workspace/bench/sweep-guest595-flat.qcow2
qemu-img convert -O qcow2 /workspace/bench/guest.qcow2 /workspace/bench/sweep-guest580.qcow2
sed 's@^image=.*@image=/workspace/bench/sweep-guest595-flat.qcow2@' /workspace/bench/guest-595.91.07.qcow2.STAGED >/workspace/bench/sweep-guest595-flat.qcow2.STAGED
printf 'version=580.159.04\nimage=/workspace/bench/sweep-guest580.qcow2\nbase=standalone-copy\n' >/workspace/bench/sweep-guest580.qcow2.STAGED
qemu-img info --output=json /workspace/bench/sweep-guest595-flat.qcow2 >/root/prov/sweep_flat595_identity.json
python3 -c 'import json; r=json.load(open("/root/prov/sweep_flat595_identity.json")); assert not r.get("backing-filename"),r'
echo "SWEEP_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"

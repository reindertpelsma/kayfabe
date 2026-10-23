#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/sweep_20261020_provision.log 2>&1
controller_pgid=$BASHPID
guard_pid=
cleanup_controller(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$guard_pid" ]; then kill "$guard_pid" 2>/dev/null; wait "$guard_pid" 2>/dev/null; fi
 journalctl -k -b -o short-iso-precise --no-pager >/root/prov/final_kernel.log
 echo "SWEEP_PROVISION_EXIT rc=$rc $(date -u -Is)"
}
trap cleanup_controller EXIT
echo "SWEEP_PROVISION_START own_instance=FROM_LEASE_RECEIPT $(date -u -Is)"
export KAYFABE_BRANCH=codex/bw-precomp-fields-20261022 CARGO_BUILD_JOBS=8
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
git checkout --detach 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
[ "$(git rev-parse HEAD)" = 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git rev-parse HEAD >/root/prov/sweep_product_source
git fetch --depth 1 origin 1e0ab16bb7d17875f0a41bcb4aacec2aa73b4d37
git worktree add --detach /workspace/bw-readiness 1e0ab16bb7d17875f0a41bcb4aacec2aa73b4d37
[ -z "$(git -C /workspace/bw-readiness status --porcelain --untracked-files=no)" ]
git -C /workspace/bw-readiness rev-parse HEAD >/root/prov/readiness_source
export WINVM_SSH_CONNECT_TIMEOUT=100
nohup bash /root/sweep_windows_media.sh </dev/null >/root/prov/sweep_windows_media_controller.log 2>&1 &
echo $! >/root/prov/sweep_windows_media.pid

bash scripts/bench/provision_host_driver.sh >/root/prov/driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 595.91.07 ]
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
python3 - <<'ZERO_XID'
import sys
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
assert not G.N.xid_lines(G.N.kernel_snapshot()),'unexpected pre-gate native Xid'
ZERO_XID
# Stock driver installation is a provisioning prerequisite; health must pass
# before any kayfabe gate, guest or timing is attributed to this box.
bash scripts/bench/host_preflight.sh >/root/prov/sweep_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/sweep_host_preflight.log
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/sweep_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/sweep-ctl-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/sweep-ctl-probe >/root/prov/sweep_host_die.log 2>&1
cat /root/prov/sweep_host_die.log
expected_die=$(python3 - <<'EXPECTED_DIE'
import json
from pathlib import Path
j=json.loads(Path('/root/sweep2/lease.json').read_text())
die=j['candidate_die'];assert die in ('GB202','GB203','GB205','GB206','GB207')
print(die)
EXPECTED_DIE
)
grep -Eq "PROBE_DIE .*die=${expected_die}([[:space:]]|$)" /root/prov/sweep_host_die.log
if [ "$expected_die" = GB205 ]; then
 echo AUDIT_SUBTRACT_GB205_ZBC_8162_ALREADY_MEASURED_57285340
else
 bash /root/sweep2/audit_probe.sh "$expected_die"
fi
# Measure own-die PRECOMP permission/reply. Existing audit data is subtracted.
# AUDIT_SUBTRACT GB202 ZBC/8162/GSS sizes already measured (2026-10-22 owner audit).
# GB206 also has existing tier2 identity/ZBC/host shapes and native Windows caps/census.
# For GB203 the known T/Z/G groups are subtracted; GB205 missing T/Z/G can use the bounded existing probe, not a second rental. No native caps recapture.
# The container wrapper would rent a second box and repeat those groups: not invoked.
[ -f scripts/bench/constants/run_container_probe.sh ] && echo S4_KIT_PRESENT_AUDIT_SUBTRACTED
cargo build --release -p kf-qemu --example precomp_host_read >/root/prov/precomp_build.log 2>&1
echo "PRECOMP_RUNTIME_BEGIN CAP_SYS_ADMIN=cleared_by_helper $(date -u -Is)"
precomp_rc=0
target/release/examples/precomp_host_read 0 >/root/prov/precomp_runtime.txt 2>&1 || precomp_rc=$?
echo "PRECOMP_RUNTIME_EXIT rc=$precomp_rc $(date -u -Is)" | tee /root/prov/precomp_exit.txt
[ "$precomp_rc" = 0 ] || [ "$precomp_rc" = 2 ]
# Privileged read-only measurement, separate checkout and target; never product input.
git fetch --depth 1 origin eed23319b9eb261cba6828f3fbc93056d8e15189
git worktree add --detach /workspace/bw-tier3-kit eed23319b9eb261cba6828f3fbc93056d8e15189
[ -z "$(git -C /workspace/bw-tier3-kit status --porcelain --untracked-files=no)" ]
git -C /workspace/bw-tier3-kit rev-parse HEAD >/root/prov/precomp_tier3_source
df -h /
CARGO_TARGET_DIR=/workspace/tier3-kit-target cargo build --locked --release -j8 --manifest-path /workspace/bw-tier3-kit/scripts/bench/constants/precomp-tier3/Cargo.toml >/root/prov/precomp_tier3_build.log 2>&1
sha256sum /workspace/tier3-kit-target/release/kf-precomp-tier3 >/root/prov/precomp_tier3_binary_sha256
echo "PRECOMP_TIER3_BEGIN $(date -u -Is)"
tier3_rc=0
timeout 120 /workspace/tier3-kit-target/release/kf-precomp-tier3 "$(date -u +%F)" 0 >/root/prov/precomp_tier3.txt 2>&1 || tier3_rc=$?
echo "PRECOMP_TIER3_EXIT rc=$tier3_rc $(date -u -Is)" | tee /root/prov/precomp_tier3_exit.txt
[ "$tier3_rc" = 0 ] || [ "$tier3_rc" = 2 ]
# An ordinary RM read refusal is recorded; T1 virtual derivation has no T2 dependency.
# Any new host Xid remains fatal before native negative controls.
python3 - <<'POST_PRECOMP'
import sys
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
assert not G.N.xid_lines(G.N.kernel_snapshot()),'native Xid during PRECOMP read is fatal'
POST_PRECOMP
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
[ -x /workspace/bench/kf3-bins/6e3fd3fd/qemu-system-x86_64 ]
sha256sum /workspace/bench/kf3-bins/6e3fd3fd/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
echo "SWEEP_BUILD_READY source=$(git rev-parse HEAD) $(date -u -Is)"
# No unexpected kernel fault may precede the intentionally marked native controls.
python3 - <<'PRE_NATIVE'
import sys,json,subprocess
from pathlib import Path
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
j=G.N.kernel_snapshot(); assert not G.N.xid_lines(j),'pre-native Xid is fatal'
bus=subprocess.check_output(['nvidia-smi','--query-gpu=pci.bus_id','--format=csv,noheader'],text=True).strip()
domain,busno,dev=bus.split(':');pci=f'{int(domain,16):04x}:{busno}:{dev.split(".")[0]}'
Path('/root/prov/pci.txt').write_text(pci+'\n');G.atomic_json('/root/prov/native_receipt.json',{'approved_native_xid_lines':[],'pci':pci})
PRE_NATIVE
chmod 0755 /root/sweep2/native_wrap.py
export KF_LADDER=/root/sweep2/native_wrap.py
bash scripts/fastguest/bare_metal_suite.sh capc_native595 180 >/root/prov/sweep_bare595.log 2>&1
[ ! -e /root/prov/XID_STOP ]
grep -q '^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$' /root/prov/sweep_bare595.log
python3 -E -s /root/sweep2/guard/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$controller_pgid" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/guard_watch.log 2>&1 &
guard_pid=$!
for i in $(seq 1 20); do [ -f /root/prov/native_receipt.json.ready.json ] && break; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/native_receipt.json.ready.json ]
kill -0 "$guard_pid"
bash scripts/bench/cuda_ladder.sh host capc_native595 1 >/root/prov/sweep_cuda_host.log 2>&1
python3 - <<'CUDA_CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_capc_native595_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
assert len(rows)==4 and all('verdict=PASS ' in l and ' rc=0 ' in l for l in rows)
CUDA_CHECK
# Native public kernel-GR report release: real GPU payload/GET and host event, before Windows.
# Corrected benchmark fixture only; product/QEMU remains the pinned 6e3 revision.
fixture_source=618f0431815d037d4226b642ae37e76967353e90
git fetch --depth 1 origin "$fixture_source"
git worktree add --detach /workspace/bw-native-fixture "$fixture_source"
[ "$(git -C /workspace/bw-native-fixture rev-parse HEAD)" = "$fixture_source" ]
[ -z "$(git -C /workspace/bw-native-fixture status --porcelain --untracked-files=no)" ]
(
 cd /workspace/bw-native-fixture
 df -h /
 CARGO_TARGET_DIR=/workspace/native-fixture-target cargo build --release -p kf-harness --bin kf-gr-tier
) >/root/prov/sweep_grtier_build.log 2>&1
python3 - <<'FIXTURE_RECEIPT'
import json,hashlib,subprocess
from pathlib import Path
p=Path('/workspace/native-fixture-target/release/kf-gr-tier')
j={'source':subprocess.check_output(['git','-C','/workspace/bw-native-fixture','rev-parse','HEAD'],text=True).strip(),'binary':str(p),'binary_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'product_unchanged':'6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b','separate_target':True}
Path('/root/prov/native_fixture_receipt.json').write_text(json.dumps(j,indent=2)+'\n')
FIXTURE_RECEIPT
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 /workspace/native-fixture-target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1
grep '^GR_TIER_VERDICT=PASS$' /root/prov/sweep_native_grtier.log
bash /root/sweep2/start_secondary.sh
bash /root/sweep2/build_b1.sh &
b1_build_pid=$!
bash /root/sweep_windows_install.sh
wait "$b1_build_pid"
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"

# N0 evidence is exported during the standard-VGA installation; no decision wait.
for i in $(seq 1 60); do [ -f /root/prov/N0_EXPORT_ACK ] && break; [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/N0_EXPORT_ACK ]
. /root/sweep2/common.sh
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
# The new phase authorizes one T1 field-derived ON, then B1 only on observed Code0.
run_cell(){
 local run=$1 binary=$2 source=$3 arm_rc=0
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 python3 /root/sweep2/budget_start.py "$run" || { echo "CELL_UNRUN_BUDGET $run"; return 30; }
 echo "VALIDATION_ARM_BEGIN $run $(date -u -Is)"
 bash /root/sweep2/arm.sh on "$run" "$binary" "$source" || arm_rc=$?
 echo "VALIDATION_ARM_RESULT run=$run rc=$arm_rc $(date -u -Is)"
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 [ -z "$(pgrep -x qemu-system-x86 || true)" ]
 if [ "$arm_rc" != 0 ]; then
  case "$arm_rc" in
   124|255) grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" ;;
   *) echo "ARM_HARNESS_REFUSAL_STOP run=$run rc=$arm_rc"; exit "$arm_rc" ;;
  esac
 fi
 # Expected guest failure is collected via standard VGA, never another product boot.
 bash /root/sweep2/collect_bw.sh "$run"
 echo "VALIDATION_ARM_END $run $(date -u -Is)"
}
actual_code0(){
 python3 -E -s - "$1" <<'ACTUAL_CODE0'
from pathlib import Path
import sys
sys.path.insert(0,'/root/sweep2/guard/blackwell')
from init_observer import clean,result
p=Path('/root/prov/arms')/sys.argv[1]
diag=p/'early_code0.txt'
assert clean(result(diag.read_text()),0)
assert 'WINDOWS_INIT_OBSERVER_END code0=1' in (p/'init_observer.log').read_text()
ACTUAL_CODE0
}
run_cell on_precision "$BIN" "$BASE"
if actual_code0 on_precision; then
 [ -f /root/prov/optional_b1.json ]
 mapfile -t b1 < <(python3 - <<'B1_META'
import json,re,hashlib
from pathlib import Path
j=json.loads(Path('/root/prov/optional_b1.json').read_text())
assert j.get('enabled') and j.get('tests_verified_before_arms') is True
assert re.fullmatch('[0-9a-f]{40}',j['source'])
assert re.fullmatch('/workspace/bench/kf3-bins/[a-zA-Z0-9_-]+/qemu-system-x86_64',j['binary'])
assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']
print(j['binary']);print(j['source'])
B1_META
 )
 [ "${#b1[@]}" = 2 ]
 if python3 /root/sweep2/budget_start.py on_b1; then
  run_cell on_b1 "${b1[0]}" "${b1[1]}"
 else echo OPTIONAL_B1_UNRUN_BUDGET; fi
else
 echo OPTIONAL_B1_UNRUN_NO_ACTUAL_CODE0
fi
echo "VALIDATION_FINAL_DONE $(date -u -Is)"

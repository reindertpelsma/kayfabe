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
export KAYFABE_BRANCH=codex/bw-native-edge-probe-20261022 CARGO_BUILD_JOBS=8
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/box.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
git fetch --depth 1 origin 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
git checkout --detach 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
[ "$(git rev-parse HEAD)" = 6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
git rev-parse HEAD >/root/prov/sweep_product_source
git fetch --depth 1 origin 9ff51a6e9432ed4d5f51636c7606360728de5b6c
git worktree add --detach /workspace/bw-readiness 9ff51a6e9432ed4d5f51636c7606360728de5b6c
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
# B1f profile: the pinned preflight is unchanged; its verdict is judged under the lease profile
# (sole accepted deviation: 'only 7 cores' with nproc 7; anything else still stops).
preflight_rc=0
bash scripts/bench/host_preflight.sh >/root/prov/sweep_host_preflight.log 2>&1 || preflight_rc=$?
echo "HOST_PREFLIGHT_RC=$preflight_rc"
python3 /root/sweep2/preflight_profile.py /root/prov/sweep_host_preflight.log \
 /root/sweep2/lease.json /root/prov/preflight_profile.json
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
# B1n (owner 2026-10-24): the observation pair is keyed by this probed die (pinned table).
python3 /root/sweep2/b1n_arms.py probed-die /root/prov/sweep_host_die.log "$expected_die" \
 /root/prov/probed_die.json
echo AUDIT_SUBTRACT_EXISTING_CONSTANTS_NO_RECAPTURE
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
native_raw_rc=0
bash scripts/fastguest/bare_metal_suite.sh capc_native595 180 --timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep --concurrency --defer-liveness --blockage-coverage --alias-two-vas --alias-unmap-observe --map-propagation --late-map-race --missing-page-fault --uvm-invalidate --uvm-mean --executor-vas --guest-ring-channel --dictated-ring --dictated-ring-negative --ce-client --ce-client-guest-ram --guest-ram-pin --bar1-crossing --atomics-probe --pce-mask-probe --gpga-reserve-probe --map-stress --cross-client-leak --rpc-mixed-allocs >/root/prov/sweep_bare595.log 2>&1 || native_raw_rc=$?
echo "NATIVE_RAW_FINDING rc=$native_raw_rc verdict_is_not_a_gate"
[ ! -e /root/prov/XID_STOP ]
grep '^BARE_SUITE_PASS=' /root/prov/sweep_bare595.log || echo NATIVE_RAW_SUMMARY_UNPARSED
python3 -E -s /root/sweep2/guard/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$controller_pgid" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/guard_watch.log 2>&1 &
guard_pid=$!
for i in $(seq 1 20); do [ -f /root/prov/native_receipt.json.ready.json ] && break; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/native_receipt.json.ready.json ]
kill -0 "$guard_pid"
native_cuda_rc=0
bash scripts/bench/cuda_ladder.sh host capc_native595 1 >/root/prov/sweep_cuda_host.log 2>&1 || native_cuda_rc=$?
echo "NATIVE_CUDA_FINDING rc=$native_cuda_rc verdict_is_not_a_gate"
python3 - <<'CUDA_CHECK'
from pathlib import Path
rows=[l for l in Path('/workspace/bench/cl_capc_native595_host.out').read_text().splitlines() if l.startswith('CL_ROW ')]
import json
Path('/root/prov/native_cuda_findings.json').write_text(json.dumps({'rows':rows,'gate':False,'experiment_mode':'OBSERVATION','attestation':False})+'\n')
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
native_gr_rc=0
env KF3_TRANSLATED_CE_RELAY=1 timeout 180 /workspace/native-fixture-target/release/kf-gr-tier >/root/prov/sweep_native_grtier.log 2>&1 || native_gr_rc=$?
echo "NATIVE_GR_FINDING rc=$native_gr_rc verdict_is_not_a_gate"
grep '^GR_TIER_VERDICT=' /root/prov/sweep_native_grtier.log || true
# The page is already measured for GB202 and GB206. This lease tests serving it.
# Tier 2 returned 0x1f previously; neither existing measurement is repeated.
echo PRECOMP_EXISTING_MEASUREMENTS_SUBTRACTED | tee /root/prov/precomp_tier3_exit.txt
python3 - <<'POST_PRECOMP'
import sys
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
import json
from pathlib import Path
allowed=json.loads(Path('/root/prov/native_receipt.json').read_text())['approved_native_xid_lines']
assert tuple(G.N.xid_lines(G.N.kernel_snapshot()))==tuple(allowed),'new Xid during PRECOMP read is fatal'
POST_PRECOMP
# B1b: exact 3 pinned repeats + one same-width unpinned control; verdicts are findings.
if bash /root/sweep2/build_fuzz_fixture.sh; then
 flock -o /tmp/kayfabe-fastguest.lock python3 /root/sweep2/fuzz_findings.py \
  /workspace/bw3-fuzz-target/release/kayfabe-rm-ladder /root/prov/native_fuzz_findings.json
else
 echo NATIVE_FUZZ_FIXTURE_UNRUN_BUILD_FAILURE
 python3 - <<'FUZZ_UNRUN'
from pathlib import Path
import json
Path('/root/prov/native_fuzz_findings.json').write_text(json.dumps({'records':[], 'pinned_samples':0,'native_finding':None,'continue_windows':True,'reason':'fixture build failed','experiment_mode':'OBSERVATION','attestation':False})+'\n')
FUZZ_UNRUN
fi
[ ! -e /root/prov/XID_STOP ]
touch /root/prov/native_stage_complete
bash /root/sweep2/start_secondary.sh
# The ready secondary protects the handoff from the frozen native watcher.
kill "$guard_pid"
wait "$guard_pid" || true
rm -f /root/prov/native_receipt.json.ready.json
python3 -E -s /root/sweep2/guard/blackwell/saved_xid32_observation.py  /root/prov/native_receipt.json "$controller_pgid" /root/prov/phase.json  /root/prov/XID_STOP /root/sweep2/observation_authority.json  /root/prov/observation_permit.json >/root/prov/guard_watch.log 2>&1 &
guard_pid=$!
for i in $(seq 1 15); do
 [ -f /root/prov/native_receipt.json.ready.json ] && break
 kill -0 "$guard_pid"
 sleep 1
done
[ -f /root/prov/native_receipt.json.ready.json ]
kill -0 "$guard_pid"
bash /root/sweep2/build_candidates.sh &
measured_build_pid=$!
bash /root/sweep_windows_install.sh
wait "$measured_build_pid"
b1_ready=0
if [ -f /root/prov/observed_b1_binary.json ]; then b1_ready=1; fi
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"

# N0 evidence is exported during the standard-VGA installation; no decision wait.
for i in $(seq 1 60); do [ -f /root/prov/N0_EXPORT_ACK ] && break; [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"; sleep 1; done
[ -f /root/prov/N0_EXPORT_ACK ]
. /root/sweep2/common.sh
BIN=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
# Same sealed parent and flags; exactly one binary/page candidate per arm.
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
   # B1n: a failed standard-VGA prep boot means no product boot happened; arm UNRUN.
   40) echo "CELL_UNRUN_PREP run=$run"; return 0 ;;
   *) echo "ARM_HARNESS_REFUSAL_STOP run=$run rc=$arm_rc"; exit "$arm_rc" ;;
  esac
 fi
 # Expected guest failure is collected via standard VGA, never another product boot.
 bash /root/sweep2/collect_bw.sh "$run"
 python3 -I /root/sweep2/channel_projection.py   "$VM/logs/${run}_b1_qemu.log" "/root/prov/arms/$run/rpc_timing.jsonl"   "/root/prov/arms/$run/channel_projection.json" "/root/prov/arms/$run/host_kernel.log"   "$VM/logs/${run}_run.log"   "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["pid"])'     "/root/prov/arm_${run}_process_receipt.json")"   || echo "EVIDENCE_PROJECTION_UNPARSED run=$run raw text retained on rental"
 echo "VALIDATION_ARM_END $run $(date -u -Is)"
}
mapfile -t measured < <(python3 - <<'MEASURED_META'
import json,hashlib
from pathlib import Path
j=json.loads(Path('/root/prov/measured_binary.json').read_text())
assert j['source']=='48b027267a0e05c9ffdb4a049d34f4d719fd1837'
assert j['binary']=='/workspace/bench/kf3-bins/48b02726/qemu-system-x86_64'
assert j['tests_verified_before_arms'] is True
assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']
print(j['binary']);print(j['source'])
MEASURED_META
)
b1=()
if [ "$b1_ready" = 1 ]; then
 mapfile -t b1 < <(python3 - <<'B1_META'
import hashlib,json
from pathlib import Path
j=json.loads(Path('/root/prov/observed_b1_binary.json').read_text())
assert j['source']=='cb80cbd29b7a05f904acd822f172485fd49938a9' and j['tests_passed'] is True
assert hashlib.sha256(Path(j['binary']).read_bytes()).hexdigest()==j['binary_sha256']
print(j['binary']);print(j['source'])
B1_META
 )
 [ "${#b1[@]}" = 2 ]
fi
[ "${#measured[@]}" = 2 ]
# B1n (owner 2026-10-24): the pinned plan, in priority order; every arm uses the B1 binary.
run_linux(){
 local run=$1 binary=$2 source=$3 linux_rc=0
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 echo "VALIDATION_ARM_BEGIN $run $(date -u -Is)"
 bash /root/sweep2/linux_reload.sh "$binary" "$source" || linux_rc=$?
 echo "VALIDATION_ARM_RESULT run=$run rc=$linux_rc $(date -u -Is)"
 [ ! -e /root/prov/XID_STOP ]; kill -0 "$guard_pid"
 [ -z "$(pgrep -x qemu-system-x86 || true)" ]
 python3 -I /root/sweep2/capture_pack.py "$run" || echo "CAPTURE_PACK_FAILED $run"
 echo "VALIDATION_ARM_END $run $(date -u -Is)"
}
if [ "$b1_ready" != 1 ]; then echo "CELL_UNRUN_B1_BUILD all B1n arms"; exit 31; fi
mapfile -t plan < <(python3 /root/sweep2/b1n_arms.py list)
[ "${#plan[@]}" = 5 ]
for row in "${plan[@]}"; do
 read -r run kind <<<"$row"
 if ! python3 /root/sweep2/budget_start.py "$run"; then echo "CELL_UNRUN_BUDGET $run"; continue; fi
 case "$kind" in
  windows) run_cell "$run" "${b1[0]}" "${b1[1]}" ;;
  linux) run_linux "$run" "${b1[0]}" "${b1[1]}" ;;
  *) echo "UNKNOWN_ARM_KIND $row"; exit 32 ;;
 esac
done
echo "VALIDATION_FINAL_DONE $(date -u -Is)"

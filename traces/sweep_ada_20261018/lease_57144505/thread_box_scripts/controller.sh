#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/s2_thread_controller.log 2>&1
xid_watcher_pid=
controller_exit(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$xid_watcher_pid" ]; then kill -TERM "$xid_watcher_pid" 2>/dev/null; wait "$xid_watcher_pid" 2>/dev/null; fi
 echo "SWEEP_THREAD_EXIT rc=$rc $(date -u -Is)"; exit "$rc"
}
trap controller_exit EXIT
trap 'exit 143' TERM
export PATH=/root/.cargo/bin:$PATH KF_DEVICE=kf3 CARGO_BUILD_JOBS=8
cd /root/kayfabe
echo "SWEEP_THREAD_START own_instance=57144505 $(date -u -Is)"
[ "$(cat /proc/sys/kernel/random/boot_id)" = d4ad1906-3575-4763-a4fb-c1e4ec4205f2 ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
git fetch --depth 1 origin f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea
git checkout --detach f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea
[ "$(git rev-parse HEAD)" = f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea ]
[ -z "$(git status --porcelain --untracked-files=no)" ]
# Native5 and gates were measured on the identical product/four suite files.
git diff --exit-code 31b56f52a2f1adec4ef72c7a05ec2964dd857739 f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea -- crates qemu cuda scripts/fastguest
python3 -B - <<'CHECK'
import sys,json
from pathlib import Path
sys.path.insert(0,'scripts/bench');import native_xid_guard as g
state=json.loads(Path('/root/prov/s2_accepted_xid_history.json').read_text())
assert len(state['approved_native_xid_lines'])==5 and not state.get('guest_proofs') and not state.get('completed_arms')
g.check_unchanged(g.kernel_snapshot(),state['approved_native_xid_lines'])
CHECK
python3 -B -m unittest discover -s scripts/bench/tests -p 'test_*guard*.py' >/root/prov/s2_thread_host_tests.log 2>&1
python3 -B -m unittest discover -s scripts/bench/tests -p test_xid_abort_tree.py >>/root/prov/s2_thread_host_tests.log 2>&1
bash scripts/bench/host_preflight.sh >/root/prov/s2_thread_host_preflight.log 2>&1
grep '^VERDICT=SOUND$' /root/prov/s2_thread_host_preflight.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 >/root/prov/s2_thread_build.log 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/sweep_raw_build.log 2>&1
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest595 >/root/prov/sweep_fast595_build.log 2>&1
[ "$(sed -n 's/^version=//p' /workspace/bench/fastguest595/driver.STAGED)" = 595.91.07 ]
sha256sum /workspace/bench/kf3-bins/f8432eed/qemu-system-x86_64 >/root/prov/sweep_qemu_sha256
git rev-parse HEAD >/root/prov/sweep_product_source
echo "LINUX_GUEST_START source=$(git rev-parse HEAD) native_source=31b56f52a2f1adec4ef72c7a05ec2964dd857739 $(date -u -Is)"
python3 -B scripts/bench/guarded_fast_suite.py s2r3_guest595 /root/prov/s2_accepted_xid_history.json f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea /workspace/bench/kf3-bins/f8432eed/qemu-system-x86_64 --fastguest-dir /workspace/bench/fastguest595 >/root/prov/sweep_linux_guest595.log 2>&1
grep '^FAST_SUITE_PASS=30 ' /root/prov/sweep_linux_guest595.log
python3 - <<'FREEZE'
import json
from pathlib import Path
state=json.loads(Path('/root/prov/s2_accepted_xid_history.json').read_text())
assert len(state['approved_native_xid_lines'])==5
assert len(state['accepted_xid_lines'])==10
assert [p['accepted_faults'] for p in state['guest_proofs']]==[4,1]
Path('/root/prov/s2_frozen_xid_history.json').write_text(json.dumps({'approved_native_xid_lines':state['accepted_xid_lines']}))
FREEZE
python3 -u scripts/bench/native_xid_guard.py watch /root/prov/s2_frozen_xid_history.json $$ /root/prov/native_xid_fatal.json >/root/prov/native_xid_watch.log 2>&1 &
xid_watcher_pid=$!
kill -0 "$xid_watcher_pid"
echo "LINUX_GUEST_EXIT rc=0 $(date -u -Is)"
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"
echo 'H_AD102_ON BEFORE RUN: current frozen product supports AD102 Windows580.88 ON; falsifier Code43/crash/TDR/desktop/CUDA/DX failure. PASS needs600second zeroTDRhold and real app results.'
bash /root/s2/arm.sh on ad102_on_d1 /workspace/bench/kf3-bins/f8432eed/qemu-system-x86_64 f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea
python3 /root/s2/validate_arm.py ad102_on_d1
S2_OBSERVER=1 bash /root/s2/arm.sh on ad102_obs_d1 /workspace/bench/kf3-bins/f8432eed/qemu-system-x86_64 f8432eed2d07b9e42e120aaf0fbdacc9fcb580ea
python3 /root/s2/validate_arm.py ad102_obs_d1
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/post_windows_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py check /root/prov/post_windows_kernel_snapshot.log /root/prov/s2_frozen_xid_history.json
echo "SWEEP_WINDOWS_ON_COMPLETE $(date -u -Is)"

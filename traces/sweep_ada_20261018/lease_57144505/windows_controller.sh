#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/s2_windows_controller.log 2>&1
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
xid_watcher_pid=
controller_exit(){ rc=$?; trap - EXIT; set +e; if [ -n "$xid_watcher_pid" ]; then kill -TERM "$xid_watcher_pid" 2>/dev/null; wait "$xid_watcher_pid" 2>/dev/null; fi; echo "S2_WINDOWS_EXIT rc=$rc $(date -u -Is)"; exit "$rc"; }
trap controller_exit EXIT
trap 'exit 143' TERM
echo "S2_WINDOWS_START own_instance=57144505 $(date -u -Is)"
[ "$(git rev-parse HEAD)" = 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0 ]
[ "$(cat /proc/sys/kernel/random/boot_id)" = 425c0e6f-5587-4b0e-a1f7-f446e46c1434 ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
python3 -B - <<'CHECK'
import sys,json
from pathlib import Path
sys.path.insert(0,'scripts/bench');import native_xid_guard as g
s=json.loads(Path('/root/prov/s2_accepted_xid_history.json').read_text())
assert len(s['completed_arms'])==30 and len({r['arm'] for r in s['completed_arms']})==30
assert len(s['approved_native_xid_lines'])==5 and len(s['accepted_xid_lines'])==10
assert [r['accepted_faults'] for r in s['guest_proofs']]==[4,1]
f=json.loads(Path('/root/prov/s2_frozen_xid_history.json').read_text())['approved_native_xid_lines']
assert f==s['accepted_xid_lines'];g.check_unchanged(g.kernel_snapshot(),f)
CHECK
python3 -u scripts/bench/native_xid_guard.py watch /root/prov/s2_frozen_xid_history.json $$ /root/prov/native_xid_fatal.json >/root/prov/native_xid_watch.log 2>&1 &
xid_watcher_pid=$!;kill -0 "$xid_watcher_pid"
bash /root/sweep_windows_install.sh
bash /root/sweep_windows580.sh
echo "SWEEP2_BIND_READY $(date -u -Is)"
echo 'H_AD102_ON BEFORE RUN: current frozen product supports AD102 Windows580.88 ON; falsifier Code43/crash/TDR/desktop/CUDA/DX failure. PASS needs600second zeroTDRhold and real app results.'
bash /root/s2/arm.sh on ad102_on_d1 /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
python3 /root/s2/validate_arm.py ad102_on_d1
S2_OBSERVER=1 bash /root/s2/arm.sh on ad102_obs_d1 /workspace/bench/kf3-bins/7f6f7e7a/qemu-system-x86_64 7f6f7e7a76c141522a47dc79d024d9b3fd4d35e0
python3 /root/s2/validate_arm.py ad102_obs_d1
journalctl -k -b -o short-iso-precise --no-pager >/root/prov/post_windows_kernel_snapshot.log
python3 scripts/bench/native_xid_guard.py check /root/prov/post_windows_kernel_snapshot.log /root/prov/s2_frozen_xid_history.json
echo "SWEEP_WINDOWS_ON_COMPLETE $(date -u -Is)"

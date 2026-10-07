#!/usr/bin/env bash
# On vmb, only after cand2b completed. Reuses candidate 1's versioned app bundle.
set -euo pipefail
exec > /root/prov/cand2_apps.log 2>&1
echo "START $(date -Is)"
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
grep -q '^EXIT rc=0 ' /root/prov/cand2b.log
if pgrep -x qemu-system-x86 || pgrep -x cargo || pgrep -x ninja; then
    echo "another build or guest is active"; exit 2
fi
cd /workspace/bench/cand2-display-checks
test "$(git rev-parse HEAD)" = 9d82f2598d267732c87476da27f501aaabe2c4af
git diff --quiet
git diff --cached --quiet
KB=/workspace/bench/kf3-bins/9d82f259/qemu-system-x86_64
test -x "$KB"
test ! -e /workspace/apps/results/cand2
echo "SOURCE $(git rev-parse HEAD)"
sha256sum "$KB" /opt/apps/bundle.tgz
t0=$(date '+%Y-%m-%d %H:%M:%S')
set +e
bash scripts/apps/apps_matrix.sh host cand2 all > /root/prov/cand2_apps_host.log 2>&1
echo "HOST_RC=$? $(date -Is)"
grep -a HOST_DONE /root/prov/cand2_apps_host.log
KF3_BIN=$KB KF_GUEST_IMG=/workspace/bench/guest_apps.qcow2 APPS_PER_BOOT=100 \
    bash scripts/apps/apps_matrix.sh guest cand2 all > /root/prov/cand2_apps_guest.log 2>&1
echo "GUEST_RC=$? $(date -Is)"
grep -a 'GUEST_DONE\|kf3 binary\|boot ap_' /root/prov/cand2_apps_guest.log | cut -c1-240
t1=$(date '+%Y-%m-%d %H:%M:%S')
R=/workspace/apps/results/cand2
journalctl -k --since "$t0" --until "$t1" --no-pager | grep -a 'Xid\|NVRM' > "$R/host_xid_journal.log"
python3 scripts/apps/triage.py "$R" > "$R/triage.txt" 2>&1
if [ -f "$R/guest_isolated.res" ]; then
    python3 scripts/apps/triage.py "$R/iso" guest.res > "$R/triage_isolated.txt" 2>&1
fi
python3 scripts/apps/summarize.py "$R" > "$R/summary.md" 2>&1
cat "$R/summary.md"
echo "APPS_DONE (inspect the results; EXIT alone is not a grade)"

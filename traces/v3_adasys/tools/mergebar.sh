#!/usr/bin/env bash
# Merge bar on a GA10x KVM box at the v3-adasys head: gates, KF_DEVICE=kf3 fast_suite 180 (30 arms),
# crate tests at fix AND master (--no-fail-fast, failure sets compared). Start marker + exit line.
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
O=/root/prov/mergebar; mkdir -p $O
echo "MERGEBAR_START $(date -Is)"
cd /root/kayfabe && git checkout -q -- . && git fetch -q origin v3-adasys v3 && git checkout -q -B v3-adasys origin/v3-adasys
echo "HEAD $(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
{ nvidia-smi --query-gpu=name,driver_version,pci.bus_id --format=csv,noheader; uname -r; cat /proc/cmdline
  echo "iommu_groups=$(ls /sys/kernel/iommu_groups | wc -l)"; lspci -vvv -s 00:07.0 2>/dev/null | grep -E 'DevCtl:' -A1
  dmesg | grep -i -E 'iommu|dmar|amd-vi|xid|aer' ; } > $O/platform.txt 2>&1
cat $O/platform.txt
n0=$(dmesg | wc -l)
bash scripts/bench/v3_gates.sh $O/v3_gates_fix.log > /dev/null 2>&1
grep -E "^HEAD=|GATE[0-9]+_VERDICT|V3_GATES_SUMMARY| FAIL " $O/v3_gates_fix.log
echo "KF3 binaries: $(ls /workspace/bench/kf3-bins/ 2>/dev/null | tr '\n' ' ')"
KF_DEVICE=kf3 bash scripts/fastguest/fast_suite.sh adasys_fix 180 > $O/fast_suite_fix.out 2>&1; echo "FAST_SUITE_EXIT=$?"
grep -E "rev=|FAST_SUITE_PASS|FAST_SUITE_RC|NOTRUN|FAIL |TIMEOUT|VMM_DIED" $O/fast_suite_fix.out | head -40
cp /workspace/bench/adasys_fix_suite.out $O/ 2>/dev/null
dmesg | tail -n +$((n0+1)) > $O/dmesg_during_gates_and_suite.log
echo "dmesg lines during gates+suite: $(wc -l < $O/dmesg_during_gates_and_suite.log); xid/fault: $(grep -c -i -E 'xid|fault|iommu|dmar|aer' $O/dmesg_during_gates_and_suite.log)"
for rev in fix master; do
  if [ $rev = master ]; then git checkout -q --detach origin/v3; else git checkout -q -B v3-adasys origin/v3-adasys; fi
  echo "== crate tests $rev $(git rev-parse --short=8 HEAD)"
  cargo test --workspace --no-fail-fast > $O/test_$rev.log 2>&1; echo "TEST_RC_$rev=$?"
  grep -E '^test result:' $O/test_$rev.log | awk -v r=$rev '{p+=$4; f+=$6; i+=$8} END {print "TOTAL_" r " passed=" p " failed=" f " ignored=" i}'
  grep -E '^test .* \.\.\. FAILED$' $O/test_$rev.log | sort > $O/failed_$rev.txt; echo "failed tests $rev: $(wc -l < $O/failed_$rev.txt)"; cat $O/failed_$rev.txt | head -10
  grep -E '^error(\[|:)' $O/test_$rev.log | sort | uniq -c | head -5
done
echo "== failed-test set, master vs fix:"; diff $O/failed_master.txt $O/failed_fix.txt && echo SAME_FAILURE_SET
git checkout -q -B v3-adasys origin/v3-adasys
echo "MERGEBAR_EXIT $(date -Is)"

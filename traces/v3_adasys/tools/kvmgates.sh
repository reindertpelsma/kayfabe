#!/usr/bin/env bash
# v3 gates at master then at the fix, on the KVM box, dmesg captured around each run
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
cd /root/kayfabe
for rev in master fix; do
  if [ $rev = master ]; then git checkout -q -- . && git checkout -q v3 && git reset -q --hard origin/v3; else git fetch -q origin v3-adasys && git checkout -q -B v3-adasys origin/v3-adasys; fi
  echo "== $rev HEAD $(git log --oneline -1 | cut -c1-80) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
  n0=$(dmesg | wc -l)
  bash scripts/bench/v3_gates.sh /root/prov/v3_gates_kvm_$rev.log > /dev/null 2>&1
  grep -E "^HEAD=|GATE[0-9]+_VERDICT|V3_GATES_SUMMARY| FAIL " /root/prov/v3_gates_kvm_$rev.log
  dmesg | tail -n +$((n0+1)) > /root/prov/dmesg_gates_kvm_$rev.log
  echo "dmesg lines during $rev gates: $(wc -l < /root/prov/dmesg_gates_kvm_$rev.log); xid/fault: $(grep -c -i -E 'xid|fault|iommu|dmar|aer' /root/prov/dmesg_gates_kvm_$rev.log)"
done
echo KVMGATES_DONE

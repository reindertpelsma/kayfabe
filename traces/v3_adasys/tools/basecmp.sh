#!/usr/bin/env bash
# the failing test targets at the fix's BASE (59cc98a9) vs the fix — like for like
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
O=/root/prov/mergebar; cd /root/kayfabe
echo "BASECMP_START $(date -Is)"
for rev in 59cc98a9 origin/v3-adasys; do
  git checkout -q --detach $rev; echo "== $(git rev-parse --short=8 HEAD)"
  for t in "-p kayfabe-abi --test truncated_row_reads" "-p kayfabe-isolate-host --test executor_vas_census" "-p kayfabe-isolate-host --test guest_ring_census" "-p kayfabe-isolate-host --test sysmem_notifier_node" "-p kayfabe-isolate-host --lib rm::tests::no_collision_across_the_workspace"; do
    r=$(cargo test -q $t 2>&1 | grep -E '^test result:' | tail -1 | cut -c1-70)
    echo "  [$t] $r"
  done
  for i in 1 2 3 4 5; do r=$(cargo test -q -p kayfabe-linux-raw --lib spawn_unsafe 2>&1 | grep -E '^test result:' | tail -1 | cut -c1-60); echo "  [spawn_unsafe run $i] $r"; done
done
git checkout -q -B v3-adasys origin/v3-adasys
echo "BASECMP_EXIT $(date -Is)"

#!/usr/bin/env bash
# v3 gates at the fix branch head (bare metal, this container's GPU)
set -uo pipefail
. $HOME/.cargo/env
cd /root/kayfabe && git checkout -q -- . && git fetch -q origin v3-adasys && git checkout -q -B v3-adasys origin/v3-adasys && echo "HEAD $(git log --oneline -1) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
bash scripts/bench/v3_gates.sh /root/prov/v3_gates_fix.log > /dev/null 2>&1
grep -E "^HEAD=|^GPU=|GATE[0-9]+_VERDICT|V3_GATES_SUMMARY" /root/prov/v3_gates_fix.log
echo FIXGATES_DONE

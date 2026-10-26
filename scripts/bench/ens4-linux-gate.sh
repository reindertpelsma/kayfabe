#!/usr/bin/env bash
# Run INSIDE an explicitly authorized Linux guest; no VM/host scheduling here.
# This collects a client result; the offline host-log scorer gives the gate verdict.
# One fresh boot per twin setting. Keep the CSV and validate host route/apply coverage.
set -euo pipefail
if [[ $# != 2 ]]; then
    echo 'usage: ens4-linux-gate.sh /path/to/kf-ens4-gate /guest/arm-output-directory' >&2
    exit 2
fi
ens4_binary=$1
ens4_output=$2
mkdir "$ens4_output"
set +e
"$ens4_binary" --gpu --csv "$ens4_output/latency.csv" > "$ens4_output/client.log" 2>&1
ens4_rc=$?
set -e
cat "$ens4_output/client.log"
echo "ENS4_CLIENT_EXIT=$ens4_rc"
exit "$ens4_rc"

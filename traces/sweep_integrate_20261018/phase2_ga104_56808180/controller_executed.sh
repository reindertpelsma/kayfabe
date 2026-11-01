#!/bin/bash
set -euo pipefail
mkdir -p /root/prov
exec >/root/prov/sweep_ga104_controller.log 2>&1
trap 'rc=$?; echo "GA104_CONTROLLER_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "GA104_CONTROLLER_START source=e8553e6ce7ec557aef9ff96c3c876e3060a6b9f8 $(date -u -Is)"
grep '^VERDICT=SOUND$' /root/prov/sweep_initial_preflight.log
bash /root/sweep-lease-56808180-provision.sh
bash /root/sweep-56808180-native.sh
bash /root/sweep-56808180-linux-arm.sh raw 595
echo "GA104_FIRST_REGRESSION_COMPLETE $(date -u -Is)"

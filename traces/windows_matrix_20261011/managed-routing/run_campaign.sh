#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
echo "MANAGED_CAMPAIGN_START=$(date -Is)"
trap 'rc=$?; echo "MANAGED_CAMPAIGN_EXIT=$rc at=$(date -Is)"' EXIT
bash /tmp/kf-managed-route-regression.sh > "$W/codex-managed-route-regression.log" 2>&1
bash /tmp/kf-managed-route-focus.sh > "$W/codex-managed-route-focus.log" 2>&1
bash /tmp/kf-managed-route-remaining.sh > "$W/codex-managed-route-remaining.log" 2>&1

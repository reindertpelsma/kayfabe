#!/usr/bin/env bash
set -euo pipefail
echo "SNAPSHOT_PRIVATE_DIAGNOSTIC_BATCH_START=$(date -Is)"
trap 'rc=$?; echo "SNAPSHOT_PRIVATE_DIAGNOSTIC_BATCH_EXIT=$rc at=$(date -Is)"' EXIT
set +e
python3 /tmp/kf-native-deferred-isolated.py
native_rc=$?
bash /tmp/kf-nbody-vulkan-snap-cases.sh
snapshot_rc=$?
bash /var/lib/kf-windows-20261005/codex-private-gravity-20261011/run_probe.sh
private_rc=$?
set -e
echo "SNAPSHOT_PRIVATE_CASE_STATUS native=$native_rc snapshot=$snapshot_rc private=$private_rc"
test "$native_rc" = 0 && test "$snapshot_rc" = 0 && test "$private_rc" = 0

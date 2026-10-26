#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Operates an ALREADY OWNED lease; never creates/destroys one or changes SSH admission.
# Usage: rehearse-vast.sh ID WINDOWS_SSH_ALIAS TRUSTED_HOST_ALIAS HOST_PRIVATE_DIR BUILD_DIR
# Raw capture files pass through scp -3 directly into trusted-host storage, never local disk.
# host_dir is validated locally and intentionally expanded before SSH.
# shellcheck disable=SC2029
set -euo pipefail
[[ $# == 5 ]] || { echo "usage: $0 ID WIN_ALIAS HOST_ALIAS HOST_DIR BUILD_DIR" >&2; exit 2; }
lease_id=$1
win_alias=$2
host_alias=$3
host_dir=$4
build_dir=$5
here=$(cd "$(dirname "$0")" && pwd)
boot_command='powershell.exe -NoProfile -Command '
boot_command+='"(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.Ticks"'
setup_command='powershell.exe -NoProfile -ExecutionPolicy Bypass '
setup_command+='-File C:/kfwc_wc1/rehearse.ps1 -Action Setup'
collect_command='powershell.exe -NoProfile -ExecutionPolicy Bypass '
collect_command+='-File C:/kfwc_wc1/rehearse.ps1 -Action Collect'
[[ $lease_id =~ ^[0-9]+$ && $win_alias =~ ^[A-Za-z0-9_-]+$ &&
   $host_alias =~ ^[A-Za-z0-9_-]+$ && $host_dir =~ ^/[A-Za-z0-9_/-]+$ &&
   $host_dir != *..* && $win_alias != "$host_alias" ]] || exit 2
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=15)
if [[ -n ${KFWC_SSH_CONFIG:-} ]]; then
    [[ -f $KFWC_SSH_CONFIG ]] || exit 2
    ssh_opts=(-F "$KFWC_SSH_CONFIG" "${ssh_opts[@]}")
fi
for alias in "$win_alias" "$host_alias"; do
    ssh -G "${ssh_opts[@]}" "$alias" | python3 -c '
import sys
cfg = dict(line.rstrip().split(" ", 1) for line in sys.stdin)
assert cfg["stricthostkeychecking"] in ("yes", "true")
assert cfg.get("forwardagent", "no") in ("no", "false")
'
done
vastai show instances-v1 --raw | python3 -c '
import json, sys
rows = json.load(sys.stdin)
if isinstance(rows, dict): rows = rows["instances"]
assert sum(str(r["id"]) == sys.argv[1] for r in rows) == 1, "owned id absent"
' "$lease_id"
for file in kfwc.sys kfwc_collect.exe d3d_minimal.exe SHA256SUMS; do
    [[ -f $build_dir/$file ]] || { echo "missing build file: $file" >&2; exit 2; }
done
ssh "${ssh_opts[@]}" "$host_alias" "mkdir -p '$host_dir'"
ssh "${ssh_opts[@]}" "$win_alias" \
    'powershell.exe -NoProfile -Command "New-Item -ItemType Directory -Force C:/kfwc_wc1/build"'
scp "${ssh_opts[@]}" "$build_dir/"{kfwc.sys,kfwc_collect.exe,d3d_minimal.exe,SHA256SUMS} \
    "$win_alias:C:/kfwc_wc1/build/"
scp "${ssh_opts[@]}" "$here/../install/install.ps1" "$here/rehearse.ps1" \
    "$here/driver_key.ps1" "$here/capture_dir.ps1" \
    "$win_alias:C:/kfwc_wc1/"
setup=$(ssh "${ssh_opts[@]}" "$win_alias" "$setup_command")
printf '%s\n' "$setup"
if [[ $setup == *KFWC_REBOOT_REQUIRED=1* ]]; then
    boot_before=$(ssh "${ssh_opts[@]}" "$win_alias" "$boot_command")
    boot_before=${boot_before//$'\r'/}
    [[ $boot_before =~ ^[0-9]+$ ]] || exit 1
    ssh "${ssh_opts[@]}" "$win_alias" 'shutdown.exe /r /t 3'
    # Same pinned Windows identity after its own test-signing reboot; no key reset/accept-new.
    sleep 10
    ready=0
    for ((attempt=0; attempt<60; attempt++)); do
        if boot_after=$(ssh "${ssh_opts[@]}" "$win_alias" "$boot_command" 2>/dev/null); then
            boot_after=${boot_after//$'\r'/}
            if [[ $boot_after =~ ^[0-9]+$ && $boot_after != "$boot_before" ]]; then
                ready=1
                break
            fi
        fi
        sleep 5
    done
    [[ $ready == 1 ]] || { echo 'same pinned Windows SSH did not return' >&2; exit 1; }
fi
set +e
ssh "${ssh_opts[@]}" "$win_alias" "$collect_command" \
    | ssh "${ssh_opts[@]}" "$host_alias" "cat > '$host_dir/operator.txt'"
stage_status=("${PIPESTATUS[@]}")
collect_rc=${stage_status[0]}
if [[ ${stage_status[1]} != 0 ]]; then collect_rc=${stage_status[1]}; fi
set -e
# Pull even on a discovery/workload failure, before the operator performs exact-id teardown.
ssh "${ssh_opts[@]}" "$host_alias" \
    "mkdir -p '$host_dir/out/warm' '$host_dir/out/startup1' '$host_dir/out/startup2'"
scp -3 "${ssh_opts[@]}" "$win_alias:C:/kfwc_wc1/out/manifest.json" \
    "$host_alias:$host_dir/out/manifest.json"
files=$(ssh "${ssh_opts[@]}" "$host_alias" "python3 - '$host_dir/out' --names" \
    < "$here/verify_pull.py")
while IFS= read -r file; do
    [[ -n $file ]] || { echo 'empty capture manifest' >&2; exit 1; }
    scp -3 "${ssh_opts[@]}" "$win_alias:C:/kfwc_wc1/out/$file" \
        "$host_alias:$host_dir/out/$file"
done <<< "$files"
ssh "${ssh_opts[@]}" "$host_alias" "python3 - '$host_dir/out'" < "$here/verify_pull.py"
echo "collection_exit=$collect_rc; capture hashes verified on trusted host"
exit "$collect_rc"

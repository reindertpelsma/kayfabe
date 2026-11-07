#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# esp_restore.sh -- from the DEV BOX: ssh into the RAM Linux (the UKI) on a Windows VM and put the original Windows
# boot files back on its ESP (\EFI\Microsoft\Boot\bootmgfw.efi and \EFI\BOOT\BOOTX64.EFI from \EFI\kf-backup),
# verify them by sha256 against the backup, and optionally reboot the VM into Windows.
#
#   esp_restore.sh HOST PORT KEY [--reboot]
#
# The real work is /usr/local/bin/kf-restore-esp inside the UKI (build_alpine_uki.sh writes it; the recovery
# timer calls the same program). Prints ESP_RESTORE_OK or ESP_RESTORE_FAIL. Never writes anywhere but the ESP.
set -uo pipefail
HOST=${1:?host}; PORT=${2:?port}; KEY=${3:?private key file}; REBOOT=${4:-}
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=15 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p "$PORT" "root@$HOST")
out=$("${SSH[@]}" 'kf-restore-esp; echo KF_RESTORE_RC=$?' 2>&1); echo "$out"
if ! echo "$out" | grep -q 'KF_RESTORE_RC=0'; then echo "ESP_RESTORE_FAIL"; exit 1; fi
echo "ESP_RESTORE_OK"
if [ "$REBOOT" = --reboot ]; then
  # sync, then a hard reboot through sysrq: busybox reboot works too, but the UKI has no init system to reboot through
  "${SSH[@]}" 'sync; echo s > /proc/sysrq-trigger; echo b > /proc/sysrq-trigger' >/dev/null 2>&1 || true
  echo "ESP_RESTORE_REBOOT_SENT"
fi

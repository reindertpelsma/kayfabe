# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gate_enable.ps1 -- enable the (disabled) NVIDIA display device ONCE and stamp the instant, for
# scripts/bench/box/windows_gate.sh. nv-install --no-enable left the device disabled with the driver bound and
# EnableGpuFirmware=1 written; this is the first start of nvlddmkm on kayfabe's device, so the stamp opens the
# window in which no reset may happen.
#
#   GATE_ENABLE utc=<yyyy-MM-ddTHH:mm:ssZ guest clock> id=<instance id> rc=<pnputil rc>
# Run it once per boot; it refuses an already enabled device (the gate never replays an enable).
$ErrorActionPreference = 'Continue'
$d = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue |
    Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' } | Select-Object -First 1
if (-not $d) { Write-Output 'GATE_ENABLE REFUSED no NVIDIA display device'; exit 2 }
if ($d.Problem -ne 'CM_PROB_DISABLED') {
    Write-Output ("GATE_ENABLE REFUSED the device is not disabled: status=$($d.Status) problem=$($d.Problem)")
    exit 2
}
$t = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$null = pnputil /enable-device $d.InstanceId
Write-Output ("GATE_ENABLE utc=$t id=$($d.InstanceId) rc=$LASTEXITCODE")
exit 0

# inf_ids.ps1 — which PCI device ids does the image's own NVIDIA display INF list? (die sweep, 2026-10-14)
# Prints the DEV_xxxx ids whose first hex digit pair matches $env-less prefixes below, one line per family group,
# so a rental can be chosen by device id BEFORE it is rented (a die the INF does not list cannot bind the driver).
$inf = Get-ChildItem 'C:\Windows\System32\DriverStore\FileRepository' -Recurse -Filter nv_dispi.inf -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $inf) { Write-Output 'INFIDS no nv_dispi.inf'; exit 0 }
$ids = [regex]::Matches((Get-Content $inf.FullName -Raw), 'DEV_([0-9A-Fa-f]{4})') | ForEach-Object { $_.Groups[1].Value.ToUpper() } | Sort-Object -Unique
Write-Output "INFIDS file=$($inf.FullName) count=$($ids.Count)"
foreach ($p in '1E','1F','20','21','22','23','24','25','26','27','28','2B','2C','2D','2E','2F') {
    $m = @($ids | Where-Object { $_.StartsWith($p) })
    Write-Output "INFIDS prefix=$p n=$($m.Count) $($m -join ',')"
}

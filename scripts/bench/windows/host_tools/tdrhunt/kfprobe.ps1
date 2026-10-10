param([string]$Name, [string]$Scn, [int]$Dur, [string]$Extra = "")
$o = "C:\kf\ovl-$Name.txt"
"start $(Get-Date -Format o) scenario=$Scn dur=$Dur extra=$Extra" | Out-File $o
$more = @()
if ($Extra -eq "ign") { $more = @("--ignore-support") }
& C:\kf\kf_overlayprobe.exe --scenario $Scn --duration $Dur --out "C:\kf\ovl-$Name.json" @more *>&1 | Out-File $o -Append
"EXIT $LASTEXITCODE" | Out-File $o -Append

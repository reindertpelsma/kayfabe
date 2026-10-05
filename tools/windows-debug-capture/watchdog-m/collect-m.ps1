# Offline-prepared recipe. Parent invokes this before shutting down probe M.
[CmdletBinding()]
param(
 [Parameter(Mandatory=$true)][ValidatePattern('^WATCHDOG-[0-9]{8}-[0-9]{4}\.dmp$')][string]$DumpName,
 [Parameter(Mandatory=$true)][string]$MinimumUtc
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$stage='C:\ProgramData\KayfabeDumpM'
$trusted='C:\ProgramData\KayfabeDebugViewStage'
if (-not (Test-Path -LiteralPath (Join-Path $trusted 'analyze-dump.ps1')) -or
    -not (Test-Path -LiteralPath (Join-Path $trusted 'kf-kd-bundle.zip'))) {
 $trusted='C:\ProgramData\KayfabeDumpMTools'
}
$wrapper=Join-Path $trusted 'analyze-dump.ps1'
$zip=Join-Path $trusted 'kf-kd-bundle.zip'
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $wrapper).Hash.ToLowerInvariant() -ne '7373e87e3f2681ccd223ada92f1c561e8f29b147c83c599d3865aa1a2049dc6a') { throw 'Wrapper SHA mismatch' }
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant() -ne 'a3883456c4631bb46b9091a2d3197f17984df99ba89c36c2f8e54c976052d800') { throw 'KD bundle SHA mismatch' }
$tokens=$null; $errors=$null
$null=[System.Management.Automation.Language.Parser]::ParseFile($wrapper,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Wrapper parse failure' }
$minimum=[DateTimeOffset]::Parse($MinimumUtc,[Globalization.CultureInfo]::InvariantCulture).UtcDateTime
$dump=Get-Item -LiteralPath (Join-Path 'C:\Windows\LiveKernelReports\WATCHDOG' $DumpName)
if ($dump.PSIsContainer -or $dump.Length -le 0 -or $dump.Length -gt 8MB -or $dump.LastWriteTimeUtc -lt $minimum) { throw 'Dump missing, oversized, or stale for M' }
if (Test-Path -LiteralPath $stage) { throw 'Refusing reused M capture directory' }
$null=New-Item -ItemType Directory -Path $stage
$before=[ordered]@{ source=$dump.FullName; bytes=$dump.Length; modified_utc=$dump.LastWriteTimeUtc.ToString('o'); sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $dump.FullName).Hash.ToLowerInvariant(); minimum_utc=$minimum.ToString('o') }
Copy-Item -LiteralPath $dump.FullName -Destination (Join-Path $stage 'watchdog.dmp')
$copy=Get-Item -LiteralPath (Join-Path $stage 'watchdog.dmp')
if ($copy.Length -ne $before.bytes -or (Get-FileHash -Algorithm SHA256 -LiteralPath $copy.FullName).Hash.ToLowerInvariant() -ne $before.sha256) { throw 'Dump changed while preserving it' }
$before | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'dump-source.json')
$lines=New-Object 'System.Collections.Generic.List[string]'
$failure=$null
try {
 & $wrapper -DumpPath $dump.FullName -Seconds 180 | ForEach-Object { $lines.Add([string]$_) }
} catch { $failure=[string]$_ }
$lines | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'wrapper.stdout')
$run=@($lines | Where-Object { $_ -match '^KAYFABE_KD_OUTPUT=C:\\ProgramData\\KayfabeKD\\run-[0-9TZ]+$' } | Select-Object -Unique)
if ($run.Count -ne 1) { throw "Could not identify one KD output directory; $failure" }
$run=$run[0].Substring('KAYFABE_KD_OUTPUT='.Length)
$missing=@(); $oversized=@()
foreach ($name in @('analysis.stdout','analysis.stderr','metadata.json','result.json','commands.txt')) {
 $path=Join-Path $run $name
 if (-not (Test-Path -LiteralPath $path)) { $missing += $name; continue }
 $file=Get-Item -LiteralPath $path
 if ($file.PSIsContainer -or $file.Length -gt 8MB) { $oversized += $name; continue }
 Copy-Item -LiteralPath $path -Destination (Join-Path $stage $name)
}
[ordered]@{ utc=[DateTime]::UtcNow.ToString('o'); wrapper_failure=$failure; kd_run=$run; missing=$missing; oversized=$oversized; kd_seconds=180; wrapper_sha256='7373e87e3f2681ccd223ada92f1c561e8f29b147c83c599d3865aa1a2049dc6a' } | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'receipt.json')
$rows=@()
foreach ($name in @('watchdog.dmp','dump-source.json','wrapper.stdout','analysis.stdout','analysis.stderr','metadata.json','result.json','commands.txt','receipt.json')) {
 $path=Join-Path $stage $name
 if (Test-Path -LiteralPath $path) {
  $file=Get-Item -LiteralPath $path
  if ($file.Length -gt 8MB) { throw "Export exceeds 8 MiB: $name" }
  $rows += [ordered]@{ name=$name; bytes=$file.Length; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant() }
 }
}
[ordered]@{ schema='kayfabe-private-dump-export/1'; files=$rows } | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $stage 'export.json')
[ordered]@{ stage=$stage; files=$rows.Count; wrapper_failure=$failure; oversized=$oversized; missing=$missing } | ConvertTo-Json -Depth 4
# A nonzero analysis result still leaves private diagnostic outputs for retrieval.
if ($failure -or $oversized.Count -or $missing.Count) { exit 4 }

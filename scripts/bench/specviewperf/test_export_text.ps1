# GPU-free Windows integration tests. Run through the guest agent as SYSTEM.
param([string]$ExportScript='C:\Users\Public\codex-VF1\export-text.ps1',[string]$ReportPath='C:\Users\Public\codex-VF1\export-integration-last.json')
$ErrorActionPreference='Stop'
if (-not (Test-Path -LiteralPath $ExportScript)) {throw 'EXPECTED RED: export-text.ps1 is absent'}
$root='C:\Users\Public\codex-VF1-export-test-'+[guid]::NewGuid().ToString('N')
$lock=$null
$checks=New-Object 'Collections.Generic.List[object]'
function Check([string]$Name,[bool]$Passed) {$checks.Add(@{name=$Name;passed=$Passed})}
try {
 New-Item -ItemType Directory -Path "$root\results\result_test\open","$root\results\result_test\locked" | Out-Null
 [IO.File]::WriteAllText("$root\results\result_test\open\stdout.log",'open text')
 [IO.File]::WriteAllText("$root\results\result_test\locked\stdout.log",'locked text')
 $lock=[IO.File]::Open("$root\results\result_test\locked\stdout.log",[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
 $first=(& $ExportScript -Root $root) | ConvertFrom-Json
 Check 'exclusive-file-is-reported' (@($first.errors).Count -eq 1)
 Check 'exclusive-file-name-is-preserved' (($first.errors -join '') -match 'locked')
 Check 'other-file-is-still-exported' (@($first.files).Count -eq 1 -and $first.files[0].text -eq 'open text')
 Check 'missing-table-is-allowed' ($first.table -eq '')
 $lock.Dispose();$lock=$null
 $second=(& $ExportScript -Root $root) | ConvertFrom-Json
 Check 'closed-file-becomes-readable' (@($second.files).Count -eq 2)
 Check 'completed-export-has-no-errors' (@($second.errors).Count -eq 0)
 $passed=@($checks | Where-Object {$_.passed}).Count
 $report=@{passed=$passed;failed=($checks.Count-$passed);checks=@($checks.ToArray())} | ConvertTo-Json -Depth 6 -Compress
 [IO.File]::WriteAllText($ReportPath,$report)
 $report
 if ($passed -ne $checks.Count) {throw 'Export integration checks failed'}
} finally {
 if ($null -ne $lock) {$lock.Dispose()}
 if (Test-Path -LiteralPath $root) {Remove-Item -LiteralPath $root -Recurse -Force}
}

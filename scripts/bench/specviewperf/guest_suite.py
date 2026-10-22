#!/usr/bin/env python3
"""Render a SYSTEM-side installer for an interactive SPECviewperf CLI task.

The generated installer belongs in the guest agent; the benchmark itself runs
as the logged-in standard user. Vendor binaries/data are never copied out.
"""
import argparse
import ntpath
import re
from pathlib import Path


def public_path(path):
    normalized = ntpath.normpath(path)
    if (not normalized.lower().startswith('c:\\users\\public\\') or
        any(c in normalized for c in '\r\n\x00"') or '..' in path.split('\\')):
        raise ValueError('output must be a normal path under C:\\Users\\Public')
    return normalized


def cli_arguments(workloads, iterations, result_directory):
    if not workloads or len(set(workloads)) != len(workloads):
        raise ValueError('workloads must be nonempty and unique')
    if any(not re.fullmatch(r'[a-z][a-z0-9_]*-[0-9]{2}', w) for w in workloads):
        raise ValueError('invalid workload name')
    if not 1 <= iterations <= 10:
        raise ValueError('iterations must be 1..10')
    return ['--workload', ','.join(workloads), '--iterations', str(iterations),
            '--resolution', 'FullHD', '--resultDirectory', public_path(result_directory)]


def ps_literal(value):
    return "'" + value.replace("'", "''") + "'"



def render_report(root):
    root = public_path(root)
    return r'''$ErrorActionPreference = 'Stop'
$root = ROOT
function Read-SpecSharedText([string]$Path) {
    $stream = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    $reader = New-Object IO.StreamReader($stream)
    try { $reader.ReadToEnd() } finally { $reader.Dispose() }
}
$reportRows = @(Get-ChildItem -LiteralPath "$root\results" -Filter 'stdout.log' -File -Recurse | ForEach-Object {
    $file = $_
    $text = Read-SpecSharedText $file.FullName
    $run = $file.Directory.Parent
    $runLog = Join-Path $run.FullName 'run-log.txt'
    $runText = if (Test-Path $runLog) {Read-SpecSharedText $runLog} else {''}
    foreach ($match in [regex]::Matches($text, '<Results\b[\s\S]*?</Results>')) {
        [xml]$resultXml = $match.Value
        $workload = $resultXml.Results.GetAttribute('Name')
        $statusMatches = [regex]::Matches($runText, ('\[Workload:\s*' + [regex]::Escape($workload) + '\]\s+(Completed|Failed)\b'))
        $status = if ($statusMatches.Count) {$statusMatches[$statusMatches.Count-1].Groups[1].Value} else {'Incomplete'}
        foreach ($testNode in $resultXml.Results.SelectNodes('Test')) {
            $index = [int]$testNode.GetAttribute('Index')
            $fps = [double]::Parse($testNode.GetAttribute('FPS_' + $index),[Globalization.CultureInfo]::InvariantCulture)
            if ([double]::IsNaN($fps) -or [double]::IsInfinity($fps) -or $fps -le 0) {throw 'HARNESS_DATA_ERROR: invalid FPS'}
            [pscustomobject]@{Run=$run.Name;Workload=$workload;Subtest=$index;FPS=$fps;Frames=[long]$testNode.GetAttribute('Frames');Seconds=[double]::Parse($testNode.GetAttribute('Seconds'),[Globalization.CultureInfo]::InvariantCulture);WorkloadStatus=$status}
        }
    }
})
$reportRows | Sort-Object Run,Workload,Subtest | Export-Csv -LiteralPath "$root\subtest-fps.csv" -NoTypeInformation -Encoding UTF8
$reportRows | Sort-Object Run,Workload,Subtest | Format-Table -AutoSize | Out-String -Width 240 | Set-Content "$root\subtest-fps.txt"
'TABLE_WRITTEN rows=' + $reportRows.Count + ' path=' + "$root\subtest-fps.csv"
'''.replace('ROOT', ps_literal(root))


def render_task(workloads, iterations, root):
    root = public_path(root)
    args = cli_arguments(workloads, iterations, root + '\\results')
    argument_line = ' '.join('"'+v+'"' for v in args)
    runner = r'''$ErrorActionPreference = 'Stop'
$root = ROOT
Set-Location 'C:\Program Files\SPECviewperf 15'
'SUITE_STARTED ' + (Get-Date).ToString('o') | Set-Content "$root\cli-status.log"
try {
    $process = Start-Process -FilePath '.\SPECviewperf-CLI.exe' -ArgumentList ARGUMENTS -NoNewWindow -PassThru -RedirectStandardOutput "$root\cli-stdout.log" -RedirectStandardError "$root\cli-stderr.log"
    $shell = New-Object -ComObject WScript.Shell
    $null = $shell.AppActivate($process.Id)
    $process.WaitForExit()
    $code = $process.ExitCode
    'SUITE_EXIT ' + $code + ' ' + (Get-Date).ToString('o') | Add-Content "$root\cli-status.log"
    @{exitCode=$code;ended=(Get-Date).ToString('o');workloads=WORKLOADS;iterations=ITERATIONS;resolution='FullHD'} | ConvertTo-Json | Set-Content "$root\cli-exit.json"
    REPORT_TABLE
    exit $code
} catch {
    'SUITE_EXIT HARNESS_ERROR ' + $_.Exception.Message | Add-Content "$root\cli-status.log"
    throw
}
'''.replace('ROOT', ps_literal(root)).replace('ARGUMENTS', ps_literal(argument_line))
    runner = runner.replace('WORKLOADS', '@(' + ','.join(ps_literal(w) for w in workloads) + ')')
    runner = runner.replace('ITERATIONS', str(iterations)).replace('REPORT_TABLE', render_report(root))
    return r'''$ErrorActionPreference = 'Stop'
$root = ROOT
$taskName = 'Codex-VF1-SPEC'
if ((Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue).State -eq 'Running') { throw 'HARNESS_PRECONDITION: task already running' }
if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { throw 'HARNESS_PRECONDITION: interactive desktop missing' }
New-Item -ItemType Directory -Path $root -Force | Out-Null
# Give the local standard user access to SYSTEM-created scripts and result files.
$acl = Get-Acl $root
$rule = New-Object System.Security.AccessControl.FileSystemAccessRule('kfadmin','Modify','ContainerInherit,ObjectInherit','None','Allow')
$acl.SetAccessRule($rule)
Set-Acl $root $acl
@'
RUNNER
'@ | Set-Content -LiteralPath "$root\run-suite.ps1" -Encoding UTF8
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -ExecutionPolicy Bypass -File "' + $root + '\run-suite.ps1"') -WorkingDirectory 'C:\Program Files\SPECviewperf 15'
$principal = New-ScheduledTaskPrincipal -UserId 'kfadmin' -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $taskName
'TASK_STARTED ' + $taskName + ' ' + (Get-Date).ToString('o')
'''.replace('ROOT', ps_literal(root)).replace('RUNNER', runner.rstrip())


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--workloads',help='comma-separated installed workloads')
    p.add_argument('--report-only',action='store_true',help='render a read-only table export for existing results')
    p.add_argument('--iterations',type=int,default=2)
    p.add_argument('--root',default=r'C:\Users\Public\codex-VF1')
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    if a.report_only:
        a.output.write_text(render_report(a.root))
    elif a.workloads:
        a.output.write_text(render_task(a.workloads.split(','),a.iterations,a.root))
    else:
        p.error('--workloads or --report-only is required')


if __name__=='__main__':main()

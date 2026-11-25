# Crash evidence of Premiere Pro since $SINCE (UTC, ISO), run by adobe_part2/drive.py (lib.ps1 prepended). Reads only. Dumps stay in the guest: the
# output has sizes, the Application 1000 text, WER report headers, and for each Premiere SentryIO minidump the exception code, the faulting address,
# the module that contains it and the crashing thread's name from the report breadcrumbs.
$R = Init-Run 'crash_evidence'
$R.outputs = @()
$out = New-Object System.Collections.ArrayList
function O($s) { [void]$script:out.Add([string]$s) }
function DumpExc($path) {
  $b = [IO.File]::ReadAllBytes($path)
  if ($b.Length -lt 32 -or [Text.Encoding]::ASCII.GetString($b, 0, 4) -ne 'MDMP') { return 'not an MDMP file' }
  $n = [BitConverter]::ToUInt32($b, 8); $dir = [BitConverter]::ToUInt32($b, 12)
  $res = @(); $mods = @()
  for ($i = 0; $i -lt $n; $i++) {
    $e = $dir + 12 * $i; $type = [BitConverter]::ToUInt32($b, $e); $rva = [BitConverter]::ToUInt32($b, $e + 8)
    if ($type -eq 6) { $code = [BitConverter]::ToUInt32($b, $rva + 8); $addr = [BitConverter]::ToUInt64($b, $rva + 24); $res += ('exception_code=0x{0:x8} address=0x{1:x}' -f $code, $addr); $script:fa = $addr }
    if ($type -eq 4) {
      $cnt = [BitConverter]::ToUInt32($b, $rva)
      for ($m = 0; $m -lt $cnt; $m++) { $o = $rva + 4 + 108 * $m; $base = [BitConverter]::ToUInt64($b, $o); $size = [BitConverter]::ToUInt32($b, $o + 8); $nr = [BitConverter]::ToUInt32($b, $o + 20)
        $len = [BitConverter]::ToUInt32($b, $nr); $nm = [Text.Encoding]::Unicode.GetString($b, $nr + 4, $len); $mods += [pscustomobject]@{ base = $base; end = $base + $size; name = Split-Path $nm -Leaf } }
    }
  }
  $hit = $mods | Where-Object { $script:fa -ge $_.base -and $script:fa -lt $_.end } | Select-Object -First 1
  $res += ('module=' + $(if ($hit) { '{0}+0x{1:x}' -f $hit.name, ($script:fa - $hit.base) } else { 'none' }))
  return ($res -join ' ')
}
try {
  $t = [datetime]::Parse($SINCE, $null, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)
  O ('== now ' + (Get-Date).ToUniversalTime().ToString('s') + 'Z, since ' + $t.ToString('s') + 'Z')
  O '== Application 1000/1001/1002'
  foreach ($e in (Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1000, 1001, 1002; StartTime = $t.ToLocalTime() } -ErrorAction SilentlyContinue | Select-Object -First 30)) {
    $m = ($e.Message -replace '\s+', ' '); O ('{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $e.TimeCreated.ToUniversalTime(), $e.Id, $m.Substring(0, [Math]::Min(500, $m.Length)))
  }
  O '== System display/TDR events'
  foreach ($e in (Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $t.ToLocalTime() } -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 41, 4101, 4116, 13, 14, 153 -or $_.ProviderName -match 'nvlddmkm|Display' } | Select-Object -First 30)) {
    O ('{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $e.TimeCreated.ToUniversalTime(), $e.Id, $e.ProviderName)
  }
  O '== LiveKernelReports'
  Get-ChildItem C:\Windows\LiveKernelReports -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | ForEach-Object { O ('{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.LastWriteTimeUtc, $_.FullName, $_.Length) }
  O '== WER reports (Report.wer header lines)'
  foreach ($f in (Get-ChildItem 'C:\ProgramData\Microsoft\Windows\WER' -Recurse -Filter Report.wer -Force -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t })) {
    O ('-- {0:s}Z {1}' -f $f.LastWriteTimeUtc, $f.Directory.Name)
    Get-Content $f.FullName -ErrorAction SilentlyContinue | Where-Object { $_ -match '^(EventType|AppName|AppPath|Sig\[\d\]\.Value|FriendlyEventName|ReportDescription)' -and $_ -notmatch '\\Users\\' } | Select-Object -First 14 | ForEach-Object { O ('   ' + $_) }
  }
  O '== LocalDumps (C:\kfapps\dumps)'
  Get-ChildItem C:\kfapps\dumps -Force -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | ForEach-Object { O ('{0:s}Z {1} {2}' -f $_.LastWriteTimeUtc, $_.Length, $_.Name); try { O ('   ' + (DumpExc $_.FullName)) } catch { O ('   parse error ' + $_.Exception.Message) } }
  O '== Premiere SentryIO'
  foreach ($db in (Get-ChildItem 'C:\Users\*\AppData\Local\Temp\Adobe\Premiere Pro' -Recurse -Directory -Filter SentryIO-db -Force -ErrorAction SilentlyContinue)) {
    $lc = Join-Path $db.FullName 'last_crash'; if (Test-Path $lc) { O ('last_crash = ' + (Get-Content $lc -Raw).Trim()) }
    foreach ($f in (Get-ChildItem $db.FullName -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | Sort-Object LastWriteTimeUtc)) {
      O ('-- {0:s}Z {1,10} {2}' -f $f.LastWriteTimeUtc, $f.Length, $f.Name)
      if ($f.Extension -eq '.dmp') { try { O ('   ' + (DumpExc $f.FullName)) } catch { O ('   parse error ' + $_.Exception.Message) } }
      elseif ($f.Length -gt 100000 -and $f.Length -le 80MB) {
        $s = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($f.FullName))
        foreach ($mm in [regex]::Matches($s, 'Unhandled exception[^\x00]{0,80}')) { O ('   breadcrumb: ' + ($mm.Value -replace '[^\x20-\x7e]', '.')) }
        foreach ($mm in [regex]::Matches($s, '"gpu-renderer"\s*:\s*"[^"]*"')) { O ('   ' + $mm.Value) }
      }
    }
  }
  $R.lines = $out.Count
  $out | Set-Content "$D\evidence.txt"
} catch { $R.error = $_.Exception.Message; $out | Set-Content "$D\evidence.txt" }
Finish $R

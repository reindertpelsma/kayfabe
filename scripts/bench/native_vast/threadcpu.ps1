# threadcpu.ps1 [-Secs 10] [-Out file] : per-thread CPU of Heaven.exe over -Secs (percent of one core) plus per-vCPU totals. Run over ssh while a benchmark is in its steady state.
param([int]$Secs = 10, [string]$Out = 'C:\bench\threadcpu.txt')
$p = Get-Process Heaven -ErrorAction Stop
$t0 = @{}; foreach ($t in $p.Threads) { $t0[$t.Id] = $t.TotalProcessorTime.TotalSeconds }
$cpu0 = $p.TotalProcessorTime.TotalSeconds
$c = Get-Counter '\Processor(*)\% Processor Time' -SampleInterval $Secs -MaxSamples 1
$p.Refresh()
$o = @("heaven_total_cpu_pct_of_one_core=$([math]::Round(($p.TotalProcessorTime.TotalSeconds-$cpu0)/$Secs*100,1))")
$o += ($c.CounterSamples | ForEach-Object { "vcpu $($_.InstanceName)=$([math]::Round($_.CookedValue))%" })
$rows = foreach ($t in $p.Threads) { if ($t0.ContainsKey($t.Id)) { [pscustomobject]@{Tid=$t.Id; Pct=[math]::Round(($t.TotalProcessorTime.TotalSeconds-$t0[$t.Id])/$Secs*100,1)} } }
$o += ($rows | Sort-Object Pct -Descending | Select-Object -First 8 | ForEach-Object { "thread $($_.Tid) $($_.Pct)%" })
$o | Set-Content $Out

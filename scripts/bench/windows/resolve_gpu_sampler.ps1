# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sampler.ps1 <out.csv> <stopfile> : once per second, per-engine GPU utilisation of the Resolve process (WDDM 'GPU Engine' counters) + dedicated memory
param($out,$stop)
$ErrorActionPreference='SilentlyContinue'
"epoch,iso,pid,e3d,compute,copy,vdec,venc,ofa,mem_pid_mb,mem_all_mb,e3d_all,compute_all,venc_all,vdec_all" | Set-Content -Encoding ASCII $out
while(-not (Test-Path $stop)){
 $t0=Get-Date
 $rp=(Get-Process -Name Resolve -ErrorAction SilentlyContinue | Select-Object -First 1).Id
 $cs=(Get-Counter -Counter @('\GPU Engine(*)\Utilization Percentage','\GPU Process Memory(*)\Dedicated Usage')).CounterSamples
 $e=@($cs | Where-Object { $_.Path -like '*gpu engine*' }); $m=@($cs | Where-Object { $_.Path -like '*gpu process memory*' })
 $acc=@{};$all=@{}
 foreach($s in $e){
  if($s.Path -match 'pid_(\d+)_.*engtype_([a-z0-9_]+)\)'){
   $p=[int]$Matches[1];$t=$Matches[2] -replace '_\d+$',''
   if($t -in 'videodecode'){$t='vdec'}elseif($t -eq 'videoencode'){$t='venc'}elseif($t -like 'compute*'){$t='compute'}
   $all[$t]+=$s.CookedValue
   if($p -eq $rp){$acc[$t]+=$s.CookedValue}
  }
 }
 $mp=0;$ma=0
 foreach($s in $m){ if($s.Path -match 'pid_(\d+)_'){ $ma+=$s.CookedValue; if([int]$Matches[1] -eq $rp){$mp+=$s.CookedValue} } }
 $now=(Get-Date).ToUniversalTime()
 $ep=([DateTimeOffset]$now).ToUnixTimeMilliseconds()/1000.0
 ('{0:F3},{1},{2},{3:F1},{4:F1},{5:F1},{6:F1},{7:F1},{8:F1},{9:F0},{10:F0},{11:F1},{12:F1},{13:F1},{14:F1}' -f $ep,$now.ToString('o'),$rp,(0+$acc['3d']),(0+$acc['compute']),(0+$acc['copy']),(0+$acc['vdec']),(0+$acc['venc']),(0+$acc['ofa']),($mp/1MB),($ma/1MB),(0+$all['3d']),(0+$all['compute']),(0+$all['venc']),(0+$all['vdec'])) | Add-Content -Encoding ASCII $out
 $d=1000-((Get-Date)-$t0).TotalMilliseconds; if($d -gt 50){Start-Sleep -Milliseconds $d}
}

$g='C:\kfapps\gravitymark\GravityMark\bin'; $o='C:\kf\out\gmdiag'; New-Item -ItemType Directory -Force $o | Out-Null; Remove-Item "$o\*" -Force -ErrorAction SilentlyContinue
$variants=@(
 @('v1_nobench_1000', @('-vk','-asteroids','1000','-count','1','-close','1','-width','1280','-height','720','-fullscreen','0','-image',"$o\v1.png")),
 @('v2_bench_temporal0_1000', @('-vk','-benchmark','1','-temporal','0','-asteroids','1000','-count','1','-close','1','-width','1280','-height','720','-fullscreen','0','-image',"$o\v2.png")))
foreach($v in $variants){
 $t0=Get-Date; $p=Start-Process "$g\GravityMark.exe" -ArgumentList $v[1] -WorkingDirectory $g -PassThru
 $done=$false
 while(((Get-Date)-$t0).TotalSeconds -lt 150){ Start-Sleep 5; if($p.HasExited){$done=$true;break} }
 $cpu=$p.CPU
 "{0} exited={1} secs={2:N0} cpu_s={3:N1} image={4}" -f $v[0],$done,((Get-Date)-$t0).TotalSeconds,$cpu,(Test-Path ("$o\"+$v[0].Substring(0,2)+".png")) | Add-Content "$o\summary.txt"
 if(-not $p.HasExited){ taskkill /PID $p.Id /T /F | Out-Null }
 Start-Sleep 3
}
'DONE' | Add-Content "$o\summary.txt"

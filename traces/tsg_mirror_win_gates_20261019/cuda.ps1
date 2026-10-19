$ErrorActionPreference='Stop'
$py='C:\Program Files\Blender Foundation\Blender 5.2\5.2\python\bin\python.exe'
'M2 CUDA BEGIN ' + (Get-Date).ToString('o') | Set-Content C:\kf-m2\cuda.log
try {
    $jobs=@()
    foreach($k in @(1,2)) {
        $seed=12345*$k
        $p=Start-Process -FilePath $py -ArgumentList @('-B','-S','C:\kf-m2\cuda_streams.py','60',"$seed") -PassThru -WindowStyle Hidden -WorkingDirectory C:\kf-m2 -RedirectStandardOutput "C:\kf-m2\cuda-$k.stdout.txt" -RedirectStandardError "C:\kf-m2\cuda-$k.stderr.txt"
        $null=$p.Handle
        $p.Id | Set-Content "C:\kf-m2\cuda-$k.pid"
        "CUDA_PROCESS k=$k pid=$($p.Id) duration_s=60 seed=$seed" | Add-Content C:\kf-m2\cuda.log
        $jobs+=,$p
    }
    $bad=0
    foreach($p in $jobs) {
        $p.WaitForExit()
        "CUDA_EXIT pid=$($p.Id) rc=$($p.ExitCode)" | Add-Content C:\kf-m2\cuda.log
        if($p.ExitCode -ne 0){$bad++}
    }
    if($bad){throw "$bad CUDA children failed"}
    'CUDA_PASS contexts=2 streams_per_context=4' | Add-Content C:\kf-m2\cuda.log
    '0' | Set-Content C:\kf-m2\cuda.rc
} catch {
    "CUDA_FAIL $_" | Add-Content C:\kf-m2\cuda.log
    '1' | Set-Content C:\kf-m2\cuda.rc
} finally {
    'M2 CUDA END ' + (Get-Date).ToString('o') | Add-Content C:\kf-m2\cuda.log
}

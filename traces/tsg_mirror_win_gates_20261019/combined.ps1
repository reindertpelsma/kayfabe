$ErrorActionPreference='Stop'
$py='C:\Program Files\Blender Foundation\Blender 5.2\5.2\python\bin\python.exe'
function Note([string]$s){ ((Get-Date).ToString('o')+' '+$s) | Add-Content C:\kf-m2\combined.log }
'M2 COMBINED' | Set-Content C:\kf-m2\combined.log
try {
    $a=@('-raster','1','-temporal','1','-fps','1','-info','1','-benchmark','1','-close','1','-count','1','-asteroids','20000','-width','1280','-height','720','-fullscreen','0','-vk','-image','C:\kf-m2\combined.png','-times','C:\kf-m2\combined.times.txt')
    Note "COMMAND GravityMark.exe $($a -join ' ')"
    $render=Start-Process -FilePath C:\kf-m2\GravityMark\bin\GravityMark.exe -ArgumentList $a -WorkingDirectory C:\kf-m2\GravityMark\bin -PassThru -RedirectStandardOutput C:\kf-m2\combined-render.stdout.txt -RedirectStandardError C:\kf-m2\combined-render.stderr.txt
    $null=$render.Handle
    $render.Id | Set-Content C:\kf-m2\combined-render.pid
    $limit=(Get-Date).AddSeconds(60)
    while(-not ((Get-Content C:\kf-m2\combined-render.stdout.txt -Raw -ErrorAction SilentlyContinue) -match 'Starting 1280x720 Vulkan CS Benchmark')) {
        if($render.HasExited -or (Get-Date) -gt $limit){throw 'Combined renderer did not become ready'}
        Start-Sleep -Milliseconds 500
    }
    Note "RENDER_READY pid=$($render.Id)"
    $jobs=@()
    foreach($k in @(1,2)) {
        $seed=11111*$k
        $p=Start-Process -FilePath $py -ArgumentList @('-B','-S','C:\kf-m2\cuda_streams.py','145',"$seed") -PassThru -WindowStyle Hidden -WorkingDirectory C:\kf-m2 -RedirectStandardOutput "C:\kf-m2\combined-cuda-$k.stdout.txt" -RedirectStandardError "C:\kf-m2\combined-cuda-$k.stderr.txt"
        $null=$p.Handle
        $p.Id | Set-Content "C:\kf-m2\combined-cuda-$k.pid"
        Note "CUDA_START k=$k pid=$($p.Id) duration_s=145 seed=$seed"
        $jobs+=,$p
    }
    $limit=(Get-Date).AddSeconds(60)
    foreach($k in @(1,2)) {
        while(-not ((Get-Content "C:\kf-m2\combined-cuda-$k.stdout.txt" -Raw -ErrorAction SilentlyContinue) -match 'CPU_CHECK PASS')) {
            if($jobs[$k-1].HasExited -or (Get-Date) -gt $limit){throw 'Combined CUDA did not pass initial CPU comparison'}
            Start-Sleep -Milliseconds 500
        }
    }
    Note 'CUDA_BOTH_READY initial_cpu_checks=PASS'
    $a=@('-y','-hide_banner','-nostdin','-nostats','-progress','C:\kf-m2\combined-encode.progress','-re','-stream_loop','-1','-i','C:\kf-m2\h264.mp4','-t','130','-an','-pix_fmt','yuv420p','-c:v','h264_nvenc','-preset','p4','C:\kf-m2\combined-encode.mp4')
    Note "COMMAND ffmpeg.exe $($a -join ' ')"
    $enc=Start-Process -FilePath C:\kf-m2\ffmpeg.exe -ArgumentList $a -WorkingDirectory C:\kf-m2 -PassThru -WindowStyle Hidden -RedirectStandardOutput C:\kf-m2\combined-encode.stdout.txt -RedirectStandardError C:\kf-m2\combined-encode.stderr.txt
    $null=$enc.Handle
    $enc.Id | Set-Content C:\kf-m2\combined-encode.pid
    Note "ENCODE_START pid=$($enc.Id) duration_s=130"
    $overlap=Get-Date
    $enc.WaitForExit()
    $elapsed=((Get-Date)-$overlap).TotalSeconds
    Note "ENCODE_END rc=$($enc.ExitCode) elapsed_s=$elapsed render_alive=$(-not $render.HasExited) cuda1_alive=$(-not $jobs[0].HasExited) cuda2_alive=$(-not $jobs[1].HasExited)"
    if($enc.ExitCode -ne 0 -or $elapsed -lt 120 -or $render.HasExited -or $jobs[0].HasExited -or $jobs[1].HasExited){throw 'Combined overlap or encode failed'}
    foreach($p in $jobs){$p.WaitForExit();Note "CUDA_END pid=$($p.Id) rc=$($p.ExitCode)";if($p.ExitCode -ne 0){throw 'Combined CUDA failed'}}
    $render.WaitForExit();Note "RENDER_END rc=$($render.ExitCode)";if($render.ExitCode -ne 0){throw 'Combined rendering failed'}
    $p=Start-Process -FilePath C:\kf-m2\ffprobe.exe -ArgumentList @('-v','error','-count_frames','-select_streams','v:0','-show_entries','stream=codec_name,nb_read_frames','-of','json','C:\kf-m2\combined-encode.mp4') -WorkingDirectory C:\kf-m2 -PassThru -WindowStyle Hidden -RedirectStandardOutput C:\kf-m2\combined-probe.stdout.txt -RedirectStandardError C:\kf-m2\combined-probe.stderr.txt
    $null=$p.Handle
    $p.WaitForExit();if($p.ExitCode -ne 0){throw 'Combined probe failed'}
    $probe=Get-Content C:\kf-m2\combined-probe.stdout.txt -Raw | ConvertFrom-Json
    if($probe.streams[0].nb_read_frames -ne '3900'){throw 'Combined encoded frame count mismatch'}
    Note 'COMBINED_PASS encoded_frames=3900 minimum_overlap_s=120 contexts=2 streams_per_context=4'
    '0' | Set-Content C:\kf-m2\combined.rc
} catch {
    Note "COMBINED_FAIL $_"
    '1' | Set-Content C:\kf-m2\combined.rc
}
Note 'COMBINED_END'

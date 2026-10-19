$ErrorActionPreference='Stop'
Set-Location C:\kf-m2
'M2 VIDEO BEGIN ' + (Get-Date).ToString('o') | Set-Content C:\kf-m2\video.log
function Run-VideoCommand {
    param([string]$Tag,[string]$Exe,[string[]]$ArgsList)
    "COMMAND $Exe $($ArgsList -join ' ')" | Add-Content C:\kf-m2\video.log
    $p=Start-Process -FilePath $Exe -ArgumentList $ArgsList -WorkingDirectory C:\kf-m2 -PassThru -RedirectStandardOutput "C:\kf-m2\$Tag.stdout.txt" -RedirectStandardError "C:\kf-m2\$Tag.stderr.txt" -WindowStyle Hidden
    $null=$p.Handle
    $p.WaitForExit()
    "RESULT $Tag rc=$($p.ExitCode)" | Add-Content C:\kf-m2\video.log
    if($p.ExitCode -ne 0){throw "$Tag failed rc=$($p.ExitCode)"}
}
try {
    foreach($codec in @('h264','hevc')) {
        Run-VideoCommand "encode-$codec" C:\kf-m2\ffmpeg.exe @('-y','-hide_banner','-nostdin','-nostats','-f','lavfi','-i','testsrc2=size=1280x720:rate=30:duration=10','-an','-pix_fmt','yuv420p','-c:v',"${codec}_nvenc",'-preset','p4',"C:\kf-m2\$codec.mp4")
        Run-VideoCommand "probe-$codec" C:\kf-m2\ffprobe.exe @('-v','error','-count_frames','-select_streams','v:0','-show_entries','stream=codec_name,width,height,nb_read_frames','-of','json',"C:\kf-m2\$codec.mp4")
        $probe=Get-Content "C:\kf-m2\probe-$codec.stdout.txt" -Raw | ConvertFrom-Json
        if($probe.streams[0].nb_read_frames -ne '300'){throw "$codec encoded frame count mismatch"}
        "FRAME_COUNT $codec PASS count=300" | Add-Content C:\kf-m2\video.log
    }
    Run-VideoCommand 'decode-cpu' C:\kf-m2\ffmpeg.exe @('-y','-hide_banner','-nostdin','-nostats','-i','C:\kf-m2\h264.mp4','-pix_fmt','yuv420p','-f','framemd5','C:\kf-m2\cpu.framemd5')
    Run-VideoCommand 'decode-nvdec' C:\kf-m2\ffmpeg.exe @('-y','-hide_banner','-nostdin','-nostats','-hwaccel','cuda','-hwaccel_output_format','cuda','-i','C:\kf-m2\h264.mp4','-vf','hwdownload,format=nv12','-pix_fmt','yuv420p','-f','framemd5','C:\kf-m2\gpu.framemd5')
    $cpu=@(Get-Content C:\kf-m2\cpu.framemd5 | Where-Object {$_ -notmatch '^#' -and $_.Trim()})
    $gpu=@(Get-Content C:\kf-m2\gpu.framemd5 | Where-Object {$_ -notmatch '^#' -and $_.Trim()})
    if($cpu.Count -ne 300 -or $gpu.Count -ne 300){throw 'Decoded frame count mismatch'}
    if(Compare-Object $cpu $gpu){throw 'CPU versus NVDEC frame hash mismatch'}
    'DECODE_CPU_COMPARE PASS frames=300 exact_per_frame_md5=300' | Add-Content C:\kf-m2\video.log
    'VIDEO_PASS' | Add-Content C:\kf-m2\video.log
    '0' | Set-Content C:\kf-m2\video.rc
} catch {
    "VIDEO_FAIL $_" | Add-Content C:\kf-m2\video.log
    '1' | Set-Content C:\kf-m2\video.rc
} finally {
    'M2 VIDEO END ' + (Get-Date).ToString('o') | Add-Content C:\kf-m2\video.log
}

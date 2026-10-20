# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([ValidateSet('Heaven','GravityMark')][string]$Client, [string]$Root, [int]$Seconds=720)
$ErrorActionPreference='Stop'
$limit=[Diagnostics.Stopwatch]::GetTimestamp()+[Diagnostics.Stopwatch]::Frequency*$Seconds
$index=0
while ([Diagnostics.Stopwatch]::GetTimestamp() -lt $limit) {
    $index++
    if ($Client -eq 'Heaven') {
        $exe='C:\kf\apps\heaven\bin\Heaven.exe'
        $a=@('-project_name','Heaven','-data_path','../','-engine_config','../data/heaven_4.0.cfg','-system_script','heaven/unigine.cpp',
          '-sound_app','null','-video_app','direct3d11','-video_multisample','0','-video_fullscreen','0','-video_mode','-1',
          '-video_width','1280','-video_height','720','-extern_define','PHORONIX,RELEASE,LANGUAGE_EN,QUALITY_HIGH,TESSELLATION_NORMAL')
    } else {
        $exe='C:\kf\apps\gravitymark\GravityMark\bin\GravityMark.exe'
        $a=@('-raster','1','-temporal','1','-fps','1','-info','1','-benchmark','1','-close','1','-count','1',
          '-asteroids','20000','-width','1280','-height','720','-fullscreen','0','-vk')
    }
    $p=Start-Process -FilePath $exe -ArgumentList $a -WorkingDirectory (Split-Path $exe) -PassThru `
      -RedirectStandardOutput "$Root\$Client-$index.stdout.txt" -RedirectStandardError "$Root\$Client-$index.stderr.txt"
    $null=$p.Handle
    "START`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())`t$Client`t$($a -join ' ')" | Add-Content "$Root\life-$Client.tsv"
    while (-not $p.HasExited -and [Diagnostics.Stopwatch]::GetTimestamp() -lt $limit) { Start-Sleep -Seconds 1; $p.Refresh() }
    if ($p.HasExited) {
        "END`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())`t$($p.ExitCode)" | Add-Content "$Root\life-$Client.tsv"
        if ($p.ExitCode -ne 0) { throw "$Client child failed with $($p.ExitCode)" }
    } else {
        # Leave our exact PID alive for terminal-silence capture; VM clean shutdown ends it.
        "ALIVE`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())" | Add-Content "$Root\life-$Client.tsv"
        break
    }
}

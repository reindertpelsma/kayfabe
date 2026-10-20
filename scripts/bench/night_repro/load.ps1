# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param([ValidateSet('Heaven','Heaven2','GravityMark','Blender','Edge')][string]$Client, [string]$Root, [int]$Seconds=720,[string]$Pair='heaven-gravity')
$ErrorActionPreference='Stop'
$limit=[Diagnostics.Stopwatch]::GetTimestamp()+[Diagnostics.Stopwatch]::Frequency*$Seconds
$index=0
if($Pair -ne 'heaven-gravity') {
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class N1OwnedWindow {
 [DllImport("user32.dll",SetLastError=true)] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int cx,int cy,uint flags);
}
'@
}
while ([Diagnostics.Stopwatch]::GetTimestamp() -lt $limit) {
    $index++
    if ($Client -in @('Heaven','Heaven2')) {
        $exe='C:\kf\apps\heaven\bin\Heaven.exe'
        $a=@('-project_name','Heaven','-data_path','../','-engine_config','../data/heaven_4.0.cfg','-system_script','heaven/unigine.cpp',
          '-sound_app','null','-video_app','direct3d11','-video_multisample','0','-video_fullscreen','0','-video_mode','-1',
          '-video_width','1280','-video_height','720','-extern_define','PHORONIX,RELEASE,LANGUAGE_EN,QUALITY_HIGH,TESSELLATION_NORMAL')
    } elseif($Client -eq 'GravityMark') {
        $exe='C:\kf\apps\gravitymark\GravityMark\bin\GravityMark.exe'
        $a=@('-raster','1','-temporal','1','-fps','1','-info','1','-benchmark','1','-close','1','-count','1',
          '-asteroids','20000','-width','1280','-height','720','-fullscreen','0','-vk')
    }
    if($Client -eq 'Blender') {
        $exe='C:\Program Files\Blender Foundation\Blender 5.2\blender.exe'
        $a=@('--factory-startup','--python','C:\kf\n1\viewport.py','--', $Root)
    }
    if($Client -eq 'Edge') {
        $exe='C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
        $a=@('--no-first-run',"--user-data-dir=$Root\edge-profile",'--app=file:///C:/kf/n1/webgl.html',
            '--window-size=960,720','--window-position=800,300','--disable-backgrounding-occluded-windows','--disable-renderer-backgrounding')
    }
    $p=Start-Process -FilePath $exe -ArgumentList $a -WorkingDirectory (Split-Path $exe) -PassThru `
      -RedirectStandardOutput "$Root\$Client-$index.stdout.txt" -RedirectStandardError "$Root\$Client-$index.stderr.txt"
    $null=$p.Handle
    "START`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())`t$Client`t$($a -join ' ')" | Add-Content "$Root\life-$Client.tsv"
    if($Pair -ne 'heaven-gravity') {
        for($i=0;$i -lt 100 -and -not $p.HasExited -and $p.MainWindowHandle -eq [IntPtr]::Zero;$i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
        if(-not $p.HasExited -and $p.MainWindowHandle -ne [IntPtr]::Zero) {
            $x=if($Client -eq 'Heaven'){0}else{600}; $y=if($Client -eq 'Heaven'){0}else{300}
            $ok=[N1OwnedWindow]::SetWindowPos($p.MainWindowHandle,[IntPtr]::Zero,$x,$y,0,0,0x4015)
            "MOVE client=$Client pid=$($p.Id) x=$x y=$y ok=$ok" | Add-Content "$Root\windows.txt"
        }
    }
    if($Client -eq 'Edge') {
        # Edge may hand the window to its own browser process and exit this launcher.
        "BROWSER_LAUNCHED`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())" | Add-Content "$Root\life-$Client.tsv"
        break
    }
    while (-not $p.HasExited -and [Diagnostics.Stopwatch]::GetTimestamp() -lt $limit) { Start-Sleep -Seconds 1; $p.Refresh() }
    if ($p.HasExited) {
        "END`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())`t$($p.ExitCode)" | Add-Content "$Root\life-$Client.tsv"
        if ($p.ExitCode -ne 0) { "FAILED`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())`t$($p.ExitCode)" | Add-Content "$Root\life-$Client.tsv"; break }
    } else {
        # Leave our exact PID alive for terminal-silence capture; VM clean shutdown ends it.
        "ALIVE`t$($p.Id)`t$([Diagnostics.Stopwatch]::GetTimestamp())" | Add-Content "$Root\life-$Client.tsv"
        break
    }
}

# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_edge.ps1 -Id APP -Page webgl|webgpu|video [-Codec h264|vp9] [-Seconds 12] [-Headless 0|1] [-Port 18080]
# Browser GPU test: serve C:\kf\web\<page>.html (and a generated test video) from a loopback HttpListener,
# open Microsoft Edge (inbox) on it with a private profile, wait for the page to POST its result to /result,
# print `KFEDGE <json>`, close Edge. No internet, no external server. The page decides `ok`; the
# matrix's GPU proof is separate (the page's renderer string, and the Edge GPU process's engine counters).
param([Parameter(Mandatory = $true)][string]$Id, [Parameter(Mandatory = $true)][string]$Page,
      [string]$Codec = 'h264', [int]$Seconds = 12, [int]$Headless = 0, [int]$Port = 18080)
$ErrorActionPreference = 'Continue'
$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { Write-Output 'KFEDGE {"ok":false,"why":"msedge.exe not found"}'; exit 3 }
$web = 'C:\kf\web'
# NB: PowerShell variables are case-insensitive: `$page` would overwrite the -Page parameter (the URL became /C:/kf/web/webgl.html.html)
$pagePath = Join-Path $web ($Page + '.html')
if (-not (Test-Path $pagePath)) { Write-Output "KFEDGE {`"ok`":false,`"why`":`"no page $pagePath`"}"; exit 3 }
$media = $null
if ($Page -eq 'video') {
    $ff = 'C:\kfapps\ffmpeg\bin\ffmpeg.exe'
    if (-not (Test-Path $ff)) { Write-Output 'KFEDGE {"ok":false,"why":"ffmpeg not staged"}'; exit 3 }
    New-Item -ItemType Directory -Force -Path 'C:\kf\media' | Out-Null
    if ($Codec -eq 'vp9') { $media = 'C:\kf\media\t_vp9.webm'; $enc = @('-c:v', 'libvpx-vp9', '-b:v', '2M', '-deadline', 'realtime', '-cpu-used', '8') }
    else { $media = 'C:\kf\media\t_h264.mp4'; $enc = @('-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-profile:v', 'high', '-preset', 'veryfast', '-movflags', '+faststart') }
    if (-not (Test-Path $media)) {
        & $ff -y -hide_banner -loglevel error -f lavfi -i 'testsrc2=size=1280x720:rate=30' -t ($Seconds + 6) @enc $media 2>&1 | Out-Null
        if (-not (Test-Path $media)) { Write-Output 'KFEDGE {"ok":false,"why":"could not generate test video"}'; exit 3 }
    }
}
$outDir = "C:\kf\out\$Id"; New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$l = New-Object System.Net.HttpListener
$l.Prefixes.Add("http://127.0.0.1:$Port/")
try { $l.Start() } catch { Write-Output "KFEDGE {`"ok`":false,`"why`":`"HttpListener: $_`"}"; exit 3 }
$q = "?s=$Seconds"
if ($media) { $q += '&src=/media/' + (Split-Path $media -Leaf) }
$url = "http://127.0.0.1:$Port/$Page.html$q"
$prof = "C:\kf\edge\$Id"
Remove-Item -Recurse -Force $prof -ErrorAction SilentlyContinue
$eargs = @("--user-data-dir=$prof", '--no-first-run', '--no-default-browser-check', '--disable-sync', '--disable-extensions',
           '--autoplay-policy=no-user-gesture-required', '--ignore-gpu-blocklist', '--enable-gpu-rasterization', '--enable-unsafe-webgpu',
           '--disable-features=msEdgeSignIn,msEdgeOnRampFRE,msUndersideButton,EdgeEntraSignin', '--window-size=1280,720', '--window-position=0,0')
if ($Headless -eq 1) { $eargs += '--headless=new' }
$eargs += $url
$proc = Start-Process -FilePath $edge -ArgumentList $eargs -PassThru
$deadline = (Get-Date).AddSeconds($Seconds + 60)
$result = $null
while (-not $result -and (Get-Date) -lt $deadline) {
    $ar = $l.BeginGetContext($null, $null)
    if (-not $ar.AsyncWaitHandle.WaitOne(1000)) { continue }
    $ctx = $l.EndGetContext($ar)
    $path = $ctx.Request.Url.AbsolutePath
    $resp = $ctx.Response
    try {
        if ($path -eq "/$Page.html") {
            $b = [System.IO.File]::ReadAllBytes($pagePath); $resp.ContentType = 'text/html; charset=utf-8'; $resp.ContentLength64 = $b.Length; $resp.OutputStream.Write($b, 0, $b.Length)
        } elseif ($path -like '/media/*' -and $media) {
            $fs = [System.IO.File]::OpenRead($media); $len = $fs.Length; $start = 0; $end = $len - 1
            $rg = $ctx.Request.Headers['Range']
            if ($rg -and $rg -match 'bytes=(\d+)-(\d*)') { $start = [int64]$Matches[1]; if ($Matches[2]) { $end = [int64]$Matches[2] }; $resp.StatusCode = 206; $resp.AddHeader('Content-Range', "bytes $start-$end/$len") }
            $n = $end - $start + 1
            $resp.ContentType = $(if ($media -like '*.webm') { 'video/webm' } else { 'video/mp4' }); $resp.AddHeader('Accept-Ranges', 'bytes'); $resp.ContentLength64 = $n
            $fs.Seek($start, 'Begin') | Out-Null
            $buf = New-Object byte[] 65536; $left = $n
            while ($left -gt 0) { $r = $fs.Read($buf, 0, [int][math]::Min($buf.Length, $left)); if ($r -le 0) { break }; $resp.OutputStream.Write($buf, 0, $r); $left -= $r }
            $fs.Close()
        } elseif ($path -eq '/result' -and $ctx.Request.HttpMethod -eq 'POST') {
            $sr = New-Object System.IO.StreamReader($ctx.Request.InputStream, [System.Text.Encoding]::UTF8); $result = $sr.ReadToEnd(); $resp.StatusCode = 204
        } elseif ($path -eq '/favicon.ico') { $resp.StatusCode = 204 }
        else { $resp.StatusCode = 404 }
    } catch { }
    try { $resp.Close() } catch { }
}
try { $l.Stop() } catch { }
if ($result) {
    Set-Content -Path (Join-Path $outDir 'edge_result.json') -Value $result -Encoding UTF8
    Write-Output ('KFEDGE ' + $result)
} else {
    Write-Output 'KFEDGE {"ok":false,"why":"no result posted before the deadline"}'
}
try { if (-not $proc.HasExited) { & taskkill.exe /PID $proc.Id /T /F 2>&1 | Out-Null } } catch { }
foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue)) { if ([string]$p.CommandLine -like "*$prof*") { & taskkill.exe /PID $p.ProcessId /T /F 2>&1 | Out-Null } }
if ($result -and $result -match '"ok"\s*:\s*true') { exit 0 } else { exit 1 }

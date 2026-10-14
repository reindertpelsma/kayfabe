# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# edge_gpu_cdp.ps1 [-Url https://www.youtube.com/shorts] [-PlaySeconds 40] [-Out C:\kf\ovl\edge_gpu.txt]
# Run in the INTERACTIVE session (scheduled task /it). Starts the inbox Edge with a DevTools port on 127.0.0.1 and a private profile, plays the page,
# then opens edge://gpu in a second tab and reads its text (shadow DOM walked) through the DevTools protocol (ClientWebSocket), so
# "Direct composition / video overlay" lines of the GPU page are captured as text. Also samples the VideoDecode engine of the msedge pids.
param([string]$Url = 'https://www.youtube.com/shorts', [int]$PlaySeconds = 40, [string]$Out = 'C:\kf\ovl\edge_gpu.txt', [int]$Port = 9333)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null; '' | Set-Content $Out
function L($s) { Add-Content -Path $Out -Value $s }
$pk = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'; New-Item -Path $pk -Force | Out-Null
foreach ($kv in @(@('HideFirstRunExperience', 1), @('AutoplayAllowed', 1), @('SyncDisabled', 1), @('BrowserSignin', 0), @('DefaultBrowserSettingEnabled', 0), @('PromotionalTabsEnabled', 0), @('StartupBoostEnabled', 0))) { New-ItemProperty -Path $pk -Name $kv[0] -Value $kv[1] -PropertyType DWord -Force | Out-Null }
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
L ("edge_version=" + (Get-Item $edge).VersionInfo.ProductVersion + " url=$Url")
$prof = 'C:\kf\edge\ovlcdp'; Remove-Item -Recurse -Force $prof -ErrorAction SilentlyContinue
$null = Start-Process $edge -ArgumentList @("--user-data-dir=$prof", '--no-first-run', '--no-default-browser-check', "--remote-debugging-port=$Port", '--remote-allow-origins=*', '--window-size=1000,700', '--window-position=0,0', $Url) -PassThru
function GetPages { $all = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 15; $r = @(); foreach ($x in $all) { if ($x.type -eq 'page') { $r += $x } }; return ,$r }   # 5.1 emits the JSON array as ONE object: unroll with foreach
function CdpEval([string]$wsurl, $js, $waitMs = 15000) {
    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $null = $ws.ConnectAsync([Uri]$wsurl, [Threading.CancellationToken]::None).Wait(15000)
    $msg = (@{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $js; returnByValue = $true } } | ConvertTo-Json -Compress -Depth 5)
    $b = [Text.Encoding]::UTF8.GetBytes($msg)
    $null = $ws.SendAsync([ArraySegment[byte]]$b, 'Text', $true, [Threading.CancellationToken]::None).Wait($waitMs)
    $buf = New-Object byte[] 65536; $sb = New-Object Text.StringBuilder
    do { $r = $ws.ReceiveAsync([ArraySegment[byte]]$buf, [Threading.CancellationToken]::None); $null = $r.Wait(20000); $null = $sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $r.Result.Count)) } while (-not $r.Result.EndOfMessage)
    $ws.Dispose()
    return ($sb.ToString() | ConvertFrom-Json)
}
Start-Sleep -Seconds 12
try {
    $pages = (GetPages)
    if ($pages.Count -gt 0 -and $pages[0].url -match 'consent\.youtube') {   # EU/ES consent interstitial: accept it in this throwaway profile
        $cj = '(function(){var b=[].slice.call(document.querySelectorAll("button,input[type=submit]")).filter(function(x){return /accept all|alle akzeptieren|aceptar todo/i.test((x.getAttribute("aria-label")||"")+x.innerText+(x.value||""));})[0];if(!b)return "no accept button";b.click();return "clicked";})()'
        $cu = ""; foreach ($x in $pages) { if ($x.url -match "consent") { $cu = [string]$x.webSocketDebuggerUrl; break } }; L ("consent ws=" + ($cu -replace "[0-9a-f-]{20,}","<id>")); $o = CdpEval $cu $cj; L ("consent: " + [string]$o.result.result.value)
        Start-Sleep -Seconds 12
        $pages = (GetPages); foreach ($pg in $pages) { L ("page after consent: " + $pg.url) }
    }
} catch { L ("consent step FAILED: $_") }
$t0 = Get-Date; $vd = @()
while (((Get-Date) - $t0).TotalSeconds -lt $PlaySeconds) {
    Start-Sleep -Seconds 2
    $pids = @{}; foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue)) { $pids[[int]$p.ProcessId] = 1 }
    try { $cs = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples } catch { continue }
    foreach ($c in $cs) { if ($c.CookedValue -gt 0 -and $c.InstanceName -match '^pid_(\d+)_.*_engtype_(.+)$' -and $pids.ContainsKey([int]$Matches[1])) { $et = $Matches[2]; L ("t=" + [int]((Get-Date) - $t0).TotalSeconds + " eng pid=" + $Matches[1] + " engtype=$et util=" + [math]::Round($c.CookedValue, 2)) } }
}
try {
    $pages = (GetPages)
    foreach ($pg in $pages) { L ("page: " + $pg.url) }
    $vj = '(function(){var v=document.querySelector("video");if(!v)return "no <video> element";return JSON.stringify({paused:v.paused,t:v.currentTime,w:v.videoWidth,h:v.videoHeight,ready:v.readyState,dropped:(v.getVideoPlaybackQuality?v.getVideoPlaybackQuality().droppedVideoFrames:-1),total:(v.getVideoPlaybackQuality?v.getVideoPlaybackQuality().totalVideoFrames:-1)});})()'
    if ($pages.Count -gt 0) { $o = CdpEval ([string]$pages[0].webSocketDebuggerUrl) $vj; L ("video: " + [string]$o.result.result.value) }
    $tab = Invoke-RestMethod -Method Put -Uri "http://127.0.0.1:$Port/json/new?edge://gpu" -TimeoutSec 15
    Start-Sleep -Seconds 6
    $js = '(function(){var o=[];function w(n){if(!n)return;if(n.nodeType===3){var t=n.textContent.trim();if(t)o.push(t);}if(n.shadowRoot)w(n.shadowRoot);for(var c=n.firstChild;c;c=c.nextSibling)w(c);}w(document.body);return o.join("\n");})()'
    $o = CdpEval $tab.webSocketDebuggerUrl $js
    L '=== edge://gpu text ==='; L ([string]$o.result.result.value)
    if ($pages.Count -gt 0) { $o = CdpEval ([string]$pages[0].webSocketDebuggerUrl) $vj; L ("video after: " + [string]$o.result.result.value) }
} catch { L ("CDP FAILED: $_") }
Get-Process msedge -ErrorAction SilentlyContinue | Stop-Process -Force
L 'DONE'

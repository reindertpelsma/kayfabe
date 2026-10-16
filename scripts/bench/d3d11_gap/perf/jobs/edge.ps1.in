# job (interactive): start Edge for 60 s on a local data page, then close only the process this job started; report GPU-driver events.
$e = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"; if (-not (Test-Path $e)) { $e = "C:\Program Files\Microsoft\Edge\Application\msedge.exe" }
"EDGE exe=$e exists=$(Test-Path $e)"
$t0 = Get-Date
$p = Start-Process $e -ArgumentList @("--new-window","--user-data-dir=C:\kfperf\edge-profile","--no-first-run","--disable-features=msEdgeSignIn","data:text/html,<body style='background:%23123'><h1>kayfabe edge sanity</h1><script>setInterval(()=>{document.title=Date.now()},500)</script></body>") -PassThru
Start-Sleep -Seconds 60
"EDGE alive_after_60s=$(-not $p.HasExited) elapsed_s=$([int]((Get-Date)-$t0).TotalSeconds)"
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Get-Process msedge -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -ge $t0 } | Stop-Process -Force -ErrorAction SilentlyContinue
Get-WinEvent -FilterHashtable @{LogName="System"; Id=153,4101; StartTime=$t0} -ErrorAction SilentlyContinue | ForEach-Object { "EVT " + $_.Id + " " + $_.TimeCreated.ToString("HH:mm:ss") + " " + $_.ProviderName }
"EDGE_DONE"

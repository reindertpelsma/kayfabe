# Premiere Pro + Media Encoder rows. Row name selects the mode: premieretl (sequence's own exporter) or amematrix (Media Encoder queue).
# The neutral timeline spec (FCPXML on the data volume) is imported through ExtendScript, which a Startup stub runs; every export slice is
# verified on the host (container, codec, size, duration).
. 'C:\kfhw\lib.ps1'
$R = Init-Run $APPNAME
$sp = 'C:\Program Files\Adobe\Adobe Media Encoder 2027\MediaIO\systempresets'
try {
  $root = Data-Root
  $fcp = Join-Path $root 'timeline\timeline.fcpxml'
  if (-not (Test-Path $fcp)) { throw 'timeline spec not on the data volume' }
  $uri = 'file:///' + ($root -replace '\\', '/')
  $xml = (Get-Content $fcp -Raw) -replace 'src="(video|synthetic)/', ('src="' + $uri + '$1/')
  Set-Content "$D\timeline.fcpxml" $xml -Encoding UTF8
  $R.fcpxml_assets = ([regex]::Matches($xml, '<asset ')).Count
  $mode = if ($APPNAME -eq 'amematrix') { 'ame' } else { 'direct' }
  # job = name, preset file, extension
  $h264 = "$sp\4E49434B_48323634"; $hevc = "$sp\4A454646_48455643"; $prores = "$sp\4D584620_504D5846"
  if ($mode -eq 'direct') {
    $jobs = ,@('h264_1080', "$h264\High Quality 1080 HD.epr", 'mp4')
  } else {
    $jobs = @(@('h264_2160', "$h264\High Quality 2160 4K.epr", 'mp4'), @('h264_1080', "$h264\High Quality 1080 HD.epr", 'mp4'), @('h264_720', "$h264\High Quality 720 HD.epr", 'mp4'),
              @('hevc_2160', "$hevc\4K UHD.epr", 'mp4'), @('hevc_1080', "$hevc\HD 1080p.epr", 'mp4'), @('hevc_pq10', "$hevc\02 - Match Source - PQ.epr", 'mp4'),
              @('hevc_8k', "$hevc\8K UHD.epr", 'mp4'), @('prores_hq', "$prores\Apple ProRes 422 HQ.epr", 'mxf'))
  }
  $jl = ($jobs | ForEach-Object { '["' + $_[0] + '","' + ($_[1] -replace '\\', '/') + '","' + $_[2] + '"]' }) -join ','
  $R.jobs = @($jobs | ForEach-Object { $_[0] })
  $js = $PP_TL.Replace('@OUT@', ($D -replace '\\', '/')).Replace('@FCPXML@', ("$D\timeline.fcpxml" -replace '\\', '/')).Replace('@MODE@', $mode).Replace('@JOBS@', "[$jl]")
  Set-Content "$D\pp_run.jsx" $js
  $R.t_script = Epoch
  Run-StartupScript 'pp' 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe' 'Adobe Premiere Pro' "$D\pp_run.jsx" "$D\pp_done.txt" 420 1
  $R.summary = [string](Get-Content "$D\pp_done.txt" -Raw -ErrorAction SilentlyContinue)
  # direct exports are synchronous; Media Encoder jobs finish later: wait for every output (or the budget), recording when each appeared
  $want = @($jobs | ForEach-Object { "$($_[0]).$($_[2])" })
  $t0 = Get-Date; $seen = @{}
  while (((Get-Date) - $t0).TotalSeconds -lt 1500 -and $seen.Count -lt $want.Count) {
    foreach ($w in $want) { if (-not $seen[$w] -and (Test-Path "$D\$w")) { $f = Get-Item "$D\$w"; $seen[$w] = @{ at = [int]((Get-Date) - $t0).TotalSeconds; len = $f.Length; lw = $f.LastWriteTime } } }
    [void](Dismiss-Dialogs 'Adobe Media Encoder'); [void](Dismiss-Dialogs 'Adobe Premiere Pro'); Start-Sleep 5
    # a file that stopped growing for 20 s is complete
    foreach ($w in @($seen.Keys)) { }
  }
  Start-Sleep 20
  $R.outputs = @(); $R.job_times = @{}
  foreach ($w in $want) { if (Test-Path "$D\$w") { $f = Get-Item "$D\$w"; if ($f.Length -le 90MB) { $R.outputs += (File-Info $f.FullName) } else { $R.big_outputs = @($R.big_outputs) + ("{0}:{1}" -f $w, $f.Length) }; $R.job_times[$w] = $f.LastWriteTime.ToString('HH:mm:ss') + ' ' + $f.Length } }
  $R.outputs += (File-Info "$D\timeline.fcpxml")
  $R.quit = 'pending'
  foreach ($n in 'Adobe Premiere Pro', 'Adobe Media Encoder') { $q = Quit-App $n; $R.quit = "$($R.quit) $n=$q" }
} catch { $R.error = $_.Exception.Message; Get-Process 'Adobe Premiere Pro', 'Adobe Media Encoder' -ErrorAction SilentlyContinue | Stop-Process -Force }
Finish $R

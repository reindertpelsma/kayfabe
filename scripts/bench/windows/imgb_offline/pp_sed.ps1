# Premiere Pro Scene Edit Detection through the Startup stub (lib.ps1 Run-StartupScript); the stub must be installed (propimage_apps_b.startup_stub_setup).
# Variables: $JSXSRC (guest path of pp_sed.jsx), $RENDERER ('' = the project default, 'software' = try to switch to the software renderer through QE first, 'nosed' = hold the sequence open without Scene Edit Detection).
$R = Init-Run 'pp_sed'
$R.outputs = @()
try {
  $clip = (Find-Asset 'sintel_trailer_1080p.mp4').FullName
  $js = (Get-Content $JSXSRC -Raw).Replace('@OUT@', ($D -replace '\\', '/')).Replace('@CLIP@', ($clip -replace '\\', '/')).Replace('@RENDERER@', $RENDERER)
  Set-Content "$D\pp_run.jsx" $js
  Run-StartupScript 'pp' 'C:\Program Files\Adobe\Adobe Premiere Pro 2027\Adobe Premiere Pro.exe' 'Adobe Premiere Pro' "$D\pp_run.jsx" "$D\pp_done.txt" 600 1
  $R.summary = [string](Get-Content "$D\pp_done.txt" -Raw -ErrorAction SilentlyContinue)
  $R.shot = Shot 'pp_after'
  $R.quit = Quit-App 'Adobe Premiere Pro'
} catch { $R.error = $_.Exception.Message; $R.shot = Shot 'pp_error' }
Finish $R

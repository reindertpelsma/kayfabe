# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Guest script body for the Resolve GPU lane (scripts/bench/windows/resolve_gpu.py). STATUS: DESIGN-ONLY, 2026-11-16.
# Placed after propimage_check.PS_LIB; sampler script is substituted for __SAMPLER__. Coordinates are measured on the 1920x1080 guest
# (Resolve 21.2.0.6 free, image A); controls with UIA rectangles are found through UI Automation.
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfWin {
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern void keybd_event(byte vk,byte sc,uint f,UIntPtr e);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool f);
 [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint dx,uint dy,uint d,UIntPtr e);
}
'@
[KfWin]::SetProcessDPIAware() | Out-Null
function Focus-Win($w){
 $h=[IntPtr]$w.Current.NativeWindowHandle;$fg=[KfWin]::GetForegroundWindow();$d=0
 $ft=[KfWin]::GetWindowThreadProcessId($fg,[ref]$d);$me=[KfWin]::GetCurrentThreadId()
 [KfWin]::AttachThreadInput($me,$ft,$true) | Out-Null
 [KfWin]::keybd_event(0x12,0,0,[UIntPtr]::Zero);[KfWin]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
 [KfWin]::SetForegroundWindow($h) | Out-Null
 [KfWin]::AttachThreadInput($me,$ft,$false) | Out-Null
 ([KfWin]::GetForegroundWindow() -eq $h)
}
function Resolve-Wins(){
 foreach($pr in Get-Process -Name Resolve -ErrorAction SilentlyContinue){
  $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pr.Id)
  foreach($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond)){ $w }
 }
}
function Dump-Tree($root,$path,$max){
 $rows=New-Object System.Collections.Generic.List[string]
 $q=New-Object System.Collections.Queue; $q.Enqueue(@($root,0))
 while($q.Count -gt 0 -and $rows.Count -lt $max){
  $it=$q.Dequeue(); $el=$it[0]; $dep=$it[1]
  try{
   $c=$el.Current; $r=$c.BoundingRectangle
   $pats=($el.GetSupportedPatterns() | ForEach-Object {$_.ProgrammaticName -replace 'PatternIdentifiers.Pattern',''}) -join ','
   $rows.Add(('{0}{1}|{2}|{3}|{4},{5},{6},{7}|{8}' -f (' ' * $dep),$c.ControlType.ProgrammaticName.Replace('ControlType.',''),$c.Name,$c.AutomationId,[int]$r.X,[int]$r.Y,[int]$r.Width,[int]$r.Height,$pats))
   foreach($ch in $el.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)){ $q.Enqueue(@($ch,($dep+1))) }
  }catch{}
 }
 $rows | Set-Content -Encoding UTF8 $path
}
function Find-Els($root,$name,$type){
 $all=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
 foreach($e in $all){ try{ $c=$e.Current; if(($name -eq $null -or $c.Name -ceq $name) -and ($type -eq $null -or $c.ControlType.ProgrammaticName -eq $type)){ $e } }catch{} }
}
function Click-Rect($el){
 $r=$el.Current.BoundingRectangle
 [KfWin]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2)) | Out-Null; Start-Sleep -Milliseconds 150
 [KfWin]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfWin]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
function Click-At($x,$y){ [KfWin]::SetCursorPos([int]$x,[int]$y) | Out-Null; Start-Sleep -Milliseconds 150; [KfWin]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [KfWin]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
function Invoke-El($el){
 try{ $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); return 'invoke' }catch{}
 try{ $el.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select(); return 'select' }catch{}
 Click-Rect $el; return 'click'
}
function Main-Win(){ Resolve-Wins | Where-Object {$_.Current.AutomationId -eq 'UiMainWindow'} | Select-Object -First 1 }
function Menu-Item($name){ $m=Main-Win; Find-Els $m $name 'ControlType.MenuItem' | Select-Object -First 1 }
function Expand-El($el){ try{ $el.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand(); 'expanded' }catch{ Click-Rect $el; 'clicked' } }
function Dump-All($path,$max){
 $rows=New-Object System.Collections.Generic.List[string]
 foreach($w in (Resolve-Wins)){ $c=$w.Current; $rows.Add(("TOP|{0}|{1}|{2}|{3},{4},{5},{6}" -f $c.ControlType.ProgrammaticName,$c.Name,$c.AutomationId,[int]$c.BoundingRectangle.X,[int]$c.BoundingRectangle.Y,[int]$c.BoundingRectangle.Width,[int]$c.BoundingRectangle.Height)) }
 $rows | Set-Content -Encoding UTF8 $path
}
function Dump-Menus($path){
 $rows=New-Object System.Collections.Generic.List[string]
 foreach($w in (Resolve-Wins)){
  if($w.Current.AutomationId -eq 'UiMainWindow'){continue}
  foreach($e in $w.FindAll([System.Windows.Automation.TreeScope]::Subtree,[System.Windows.Automation.Condition]::TrueCondition)){ try{$c=$e.Current; $r=$c.BoundingRectangle; $rows.Add(("{0}|{1}|{2}|{3}|{4},{5},{6},{7}" -f $w.Current.Name,$c.ControlType.ProgrammaticName.Replace('ControlType.',''),$c.Name,$c.AutomationId,[int]$r.X,[int]$r.Y,[int]$r.Width,[int]$r.Height))}catch{} }
 }
 $rows | Set-Content -Encoding UTF8 $path
}

# --- Deliver page helpers (UIA for discovery and verification, coordinates only derived from UIA rectangles) ---
function Epoch-Now(){ ([DateTimeOffset](Get-Date).ToUniversalTime()).ToUnixTimeMilliseconds()/1000.0 }
function Vis($e){ try{ $r=$e.Current.BoundingRectangle; ($r.Width -gt 0 -and $r.X -ge -5 -and $r.Y -ge 0 -and ($r.Y+$r.Height) -le 1080) }catch{ $false } }
function All-Els(){ $m=Main-Win; if(-not $m){return @()}; @($m.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)) }
function Label-Row($els,$label,$type,$maxX){
 # the control of $type on the same row as the left-panel Text $label
 $lab=$els | Where-Object { try{ $c=$_.Current; $c.Name -ceq $label -and $c.ControlType.ProgrammaticName -eq 'ControlType.Text' -and (Vis $_) -and $c.BoundingRectangle.X -lt $maxX }catch{$false} } | Select-Object -First 1
 if(-not $lab){ return $null }
 $y=$lab.Current.BoundingRectangle.Y
 $els | Where-Object { try{ $c=$_.Current; $c.ControlType.ProgrammaticName -eq $type -and (Vis $_) -and [Math]::Abs($c.BoundingRectangle.Y-$y) -lt 8 -and $c.BoundingRectangle.X -gt $lab.Current.BoundingRectangle.X }catch{$false} } | Select-Object -First 1
}
function Val($el){ try{ $el.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value }catch{ '' } }
function Set-Combo($label,$text){
 for($try=0;$try -lt 3;$try++){
  $els=All-Els; $cb=Label-Row $els $label 'ControlType.ComboBox' 700
  if(-not $cb){ Start-Sleep 2; continue }
  if((Val $cb) -eq $text){ return $true }
  Click-Rect $cb; Start-Sleep 2
  [System.Windows.Forms.SendKeys]::SendWait($text); Start-Sleep -Milliseconds 700
  [System.Windows.Forms.SendKeys]::SendWait('{ENTER}'); Start-Sleep 3
  $els=All-Els; $cb=Label-Row $els $label 'ControlType.ComboBox' 700
  if($cb -and (Val $cb) -eq $text){ return $true }
  [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Start-Sleep 1
 }
 return $false
}
function Combo-Value($label){ $els=All-Els; $cb=Label-Row $els $label 'ControlType.ComboBox' 700; if($cb){Val $cb}else{$null} }
function Set-Edit($label,$text){
 $els=All-Els; $ed=Label-Row $els $label 'ControlType.Edit' 700
 if(-not $ed){ return $false }
 Click-Rect $ed; Start-Sleep -Milliseconds 400
 [System.Windows.Forms.SendKeys]::SendWait('^a'); Start-Sleep -Milliseconds 200
 [System.Windows.Forms.SendKeys]::SendWait($text); Start-Sleep -Milliseconds 300
 [System.Windows.Forms.SendKeys]::SendWait('{TAB}'); Start-Sleep 1
 $els=All-Els; $ed=Label-Row $els $label 'ControlType.Edit' 700
 return ($ed -and (Val $ed) -eq $text)
}
function Find-Btn($els,$name){ $els | Where-Object { try{ $c=$_.Current; $c.Name -ceq $name -and $c.ControlType.ProgrammaticName -eq 'ControlType.Button' -and (Vis $_) }catch{$false} } | Select-Object -First 1 }
function Press-Btn($name){
 $els=All-Els; $b=Find-Btn $els $name
 if(-not $b){ return $false }
 Click-Rect $b; return $true
}
function Queue-Texts(){
 $els=All-Els
 $t=$els | Where-Object { try{ $c=$_.Current; $c.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $c.Name -and $c.BoundingRectangle.X -gt 1450 -and $c.BoundingRectangle.Y -gt 100 -and $c.BoundingRectangle.Y -lt 540 }catch{$false} }
 @($t | ForEach-Object { $_.Current.Name })
}
function Wait-Until($sb,$timeout,$step){ $dl=(Get-Date).AddSeconds($timeout); while((Get-Date) -lt $dl){ if(& $sb){ return $true }; Start-Sleep $step }; return $false }

# ===================================================== flow (the plan arrives as $plan; the record is $r, the run directory $D)
$ErrorActionPreference='Continue'
$r.kind='RESOLVE_GPU'; $r.stage='init'; $r.steps=@(); $r.renders=@(); $r.log_lines=@()
function Step($n,$ok,$note){ $script:r.steps += @{n=$n;ok=[bool]$ok;note=[string]$note;t=(Epoch-Now)} }
function Prefix-Combo($label,$prefix){
 for($try=0;$try -lt 3;$try++){
  $els=All-Els; $cb=Label-Row $els $label 'ControlType.ComboBox' 700
  if(-not $cb){ Start-Sleep 2; continue }
  if((Val $cb).StartsWith($prefix)){ return $true }
  Click-Rect $cb; Start-Sleep 2
  [System.Windows.Forms.SendKeys]::SendWait($prefix); Start-Sleep -Milliseconds 700
  [System.Windows.Forms.SendKeys]::SendWait('{ENTER}'); Start-Sleep 3
  $els=All-Els; $cb=Label-Row $els $label 'ControlType.ComboBox' 700
  if($cb -and (Val $cb).StartsWith($prefix)){ return $true }
  [System.Windows.Forms.SendKeys]::SendWait('{ESC}'); Start-Sleep 1
 }
 return $false
}
function Goto-Page($page,$marker){
 $els=All-Els
 $b=$els | Where-Object { try{ $_.Current.AutomationId.EndsWith("button${page}Panel") }catch{$false} } | Select-Object -First 1
 if(-not $b){ return $false }
 try{ $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }catch{ Click-Rect $b }
 return (Wait-Until { $x=All-Els; ($x | Where-Object { try{ $_.Current.Name -ceq $marker -and (Vis $_) }catch{$false} } | Select-Object -First 1) -ne $null } 90 3)
}
function Dbl-At($x,$y){ Click-At $x $y; Start-Sleep -Milliseconds 120; Click-At $x $y }
function Win-Has($name){ $x=All-Els; ($x | Where-Object { try{ $_.Current.Name -ceq $name }catch{$false} } | Select-Object -First 1) -ne $null }

# --- samplers (guest side): WDDM per-engine counters of the Resolve process, and nvidia-smi as the guest driver reports it
$samplerText=@'
__SAMPLER__
'@
Set-Content -Encoding ASCII "$D\sampler.ps1" $samplerText
$stopFile="$D\sampler.stop"; Remove-Item $stopFile -ErrorAction SilentlyContinue
$samp=Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$D\sampler.ps1`" `"$D\engine.csv`" `"$stopFile`""
$smi=Start-Process nvidia-smi -WindowStyle Hidden -PassThru -RedirectStandardOutput "$D\smi.csv" -ArgumentList '--query-gpu=timestamp,utilization.gpu,utilization.memory,utilization.encoder,utilization.decoder,memory.used','--format=csv,noheader,nounits','-lms','1000'
$r.t0=Epoch-Now
$r.tz_offset_s=[int]([TimeZoneInfo]::Local.GetUtcOffset((Get-Date)).TotalSeconds)
Step 'samplers' $true ''
$r.grade_nodes=@()
try {

# --- Resolve: start, project manager, new project
$logPath="$env:APPDATA\Blackmagic Design\DaVinci Resolve\Support\logs\davinci_resolve.log"
$r.log_start_bytes=(Get-Item $logPath -ErrorAction SilentlyContinue).Length
$tStart=Get-Date
Start-Process 'C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe' | Out-Null
$pm=$null
$ok=Wait-Until { $script:pm=Resolve-Wins | Where-Object {$_.Current.AutomationId -eq 'UiProjectManagerDialog'} | Select-Object -First 1; $script:pm -ne $null } 300 3
Step 'project manager' $ok ''
if(-not $ok){ throw 'Resolve Project Manager never appeared' }
$new=$pm.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.Name -ceq 'New Project' -and $_.Current.AutomationId.EndsWith('btnAddProject')} | Select-Object -First 1
$new.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep 5
Focus-Win $pm | Out-Null
[System.Windows.Forms.SendKeys]::SendWait('^a'); Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait($plan.project); Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
$ok=Wait-Until { $m=Main-Win; $m -and $m.Current.Name -like "*$($plan.project)*" } 180 3
$r.window_name=(Main-Win).Current.Name
Step 'main window' $ok $r.window_name
if(-not $ok){ throw 'Resolve main window for the new project never appeared' }
Start-Sleep 8

# --- import the FCPXML timeline (File > Import > Timeline, Ctrl+Shift+I)
Focus-Win (Main-Win) | Out-Null
Click-At 1300 300; Start-Sleep 1
[System.Windows.Forms.SendKeys]::SendWait('^+i'); Start-Sleep 6
Click-At 450 515; Start-Sleep 1
[System.Windows.Forms.SendKeys]::SendWait($plan.fcpxml_guest); Start-Sleep 1
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}'); Start-Sleep 10
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
$ok=Wait-Until { Win-Has $plan.timeline } 120 4
if(-not $ok){ Click-At 1330 862; Click-At 1330 902; $ok=Wait-Until { Win-Has $plan.timeline } 60 4 }
Step 'import timeline' $ok $plan.timeline
if(-not $ok){ $r.import_unconfirmed=$true }
Start-Sleep 5

# --- grade (best effort; recorded either way): node names are read back from the node graph
if($plan.grade){
 $okc=Goto-Page 'Color' 'Scopes'
 Step 'color page' $okc ''
 if($okc){
  Start-Sleep 5
  foreach($clipX in $plan.grade_clips){
   Click-At $clipX 480; Start-Sleep 4
   Click-At 186 576; Start-Sleep 3
   foreach($f in @(@(528,648,'1.35'),@(597,956,'72'),@(133,956,'35'))){ Dbl-At $f[0] $f[1]; Start-Sleep -Milliseconds 300; [System.Windows.Forms.SendKeys]::SendWait($f[2]+'{ENTER}'); Start-Sleep 1 }
   [System.Windows.Forms.SendKeys]::SendWait('%s'); Start-Sleep 3
   foreach($f in @(@(231,648,'-900'),@(438,956,'25'))){ Dbl-At $f[0] $f[1]; Start-Sleep -Milliseconds 300; [System.Windows.Forms.SendKeys]::SendWait($f[2]+'{ENTER}'); Start-Sleep 1 }
   [System.Windows.Forms.SendKeys]::SendWait('%s'); Start-Sleep 3
   Click-At 935 576; Start-Sleep 3
   Dbl-At 102 903; Start-Sleep -Milliseconds 300; [System.Windows.Forms.SendKeys]::SendWait('0.85{ENTER}'); Start-Sleep 2
   $x=All-Els
   $nodes=@($x | Where-Object { try{ $c=$_.Current; $c.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $c.Name -match '^\d\d$' -and $c.BoundingRectangle.X -gt 1300 -and $c.BoundingRectangle.Y -lt 400 }catch{$false} } | ForEach-Object { $_.Current.Name })
   $r.grade_nodes += @{clip_x=$clipX;nodes=$nodes}
  }
  Step 'grade' $true 'nodes read back in grade_nodes'
 }
}

# --- Deliver
$okd=Goto-Page 'Deliver' 'Render Queue'
Step 'deliver page' $okd ''
if(-not $okd){ throw 'Deliver page did not open' }
$okt=Wait-Until { Win-Has $plan.timeline } 60 3
Step 'timeline on deliver page' $okt $plan.timeline
if(-not $okt){ throw 'the imported timeline is not on the Deliver page' }
Start-Sleep 5
New-Item -Force -ItemType Directory $plan.out_dir | Out-Null
$eng=Combo-Value 'Encoder'
$r.encoder_default=$eng
foreach($p in $plan.renders){
 $rr=@{id=$p.id;label=$p.label}
 $rr.fmt_ok=Set-Combo 'Format' $p.fmt
 if($p.codec){ $rr.codec_ok=Set-Combo 'Codec' $p.codec } else { $rr.codec_ok=$true }
 if($p.try_nvidia){ $rr.nvidia_encoder_selectable=Set-Combo 'Encoder' 'NVIDIA'; }
 $rr.encoder_value=Combo-Value 'Encoder'
 if($p.resolution_prefix){ $rr.res_ok=Prefix-Combo 'Resolution' $p.resolution_prefix } else { $rr.res_ok=Prefix-Combo 'Resolution' 'Timeline' }
 $rr.fmt_value=Combo-Value 'Format'; $rr.codec_value=Combo-Value 'Codec'; $rr.res_value=Combo-Value 'Resolution'
 $rr.name_ok=Set-Edit 'File Name' $p.name
 $rr.loc_ok=Set-Edit 'Location' $plan.out_dir
 $rr.queued=Press-Btn 'Add to Render Queue'
 Start-Sleep 4
 $rr.t_start=Epoch-Now
 $rr.render_pressed=Press-Btn 'Render All'
 $rr.started=Wait-Until { $x=All-Els; (Find-Btn $x 'Stop') -ne $null } 90 2
 $rr.t_started=Epoch-Now
 $rr.finished=Wait-Until { $x=All-Els; (Find-Btn $x 'Stop') -eq $null } ([int]$p.timeout_s) 3
 $rr.t_end=Epoch-Now
 if(-not $rr.finished){ Press-Btn 'Stop' | Out-Null; $rr.stopped_by_driver=$true; Start-Sleep 20 }
 Start-Sleep 3
 $rr.queue=Queue-Texts
 $rr.files=@(Get-ChildItem $plan.out_dir -Recurse -ErrorAction SilentlyContinue | Where-Object {$_.Name -like "$($p.name)*" -and -not $_.PSIsContainer} | ForEach-Object { "$($_.FullName.Substring($plan.out_dir.Length+1).Replace('\','/'))|$($_.Length)|$(Epoch $_.LastWriteTime)" })
 $r.renders += $rr
 $r.stage="render $($p.id) done"
 if(-not $rr.finished){ break }
}

# --- Resolve's own log after the start, GPU-relevant lines only
Start-Sleep 3
$lines=@()
foreach($ln in Get-Content $logPath -ErrorAction SilentlyContinue){
 $m=[regex]::Match($ln,'\| (\d{4}-\d\d-\d\d \d\d:\d\d:\d\d),\d+ \| (.*)$')
 if($m.Success){ $ts=[datetime]::ParseExact($m.Groups[1].Value,'yyyy-MM-dd HH:mm:ss',$null)
  if($ts -ge $tStart.AddSeconds(-2) -and $m.Groups[2].Value -match 'Selected compute API|compute API set|Automatic GPU Selection|Detected \d+ GPUs|NVDEC decodes|NVENC encodes|initialised as decoding|Initialized .*OpenGL|Let There Be CUDA|Initializing CUDA|Board manager thread|CPU only|No GPU|OpenCL|VRAM|GPUDetect.*ERROR|NVML'){ $lines+=$ln.Substring(0,[Math]::Min(260,$ln.Length)) } } }
$r.log_lines=@($lines | Select-Object -First 80)
$r.stage='done'
} finally {
 New-Item $stopFile -Force | Out-Null
 try{ if($smi -and -not $smi.HasExited){ Stop-Process -Id $smi.Id -Force } }catch{}
 Start-Sleep 3
}

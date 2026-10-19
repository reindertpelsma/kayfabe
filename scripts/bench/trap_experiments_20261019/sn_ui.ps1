# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_ui.ps1 -Action dump|run|click|shot|focus|launch [-Out C:\kf\TE\ui.txt]
# Runs in the INTERACTIVE session (scheduled task /it, see sn_it.ps1). UI Automation on the 3DMark Demo window:
# 3DMark Demo is a CEF window: UIA exposes no DOM, so `click` clicks at window-relative fractions (-FX -FY) and `run` clicks
# the results page's RUN AGAIN button at its measured position (0.1786, 0.845 of the 1400x880 window).
param([string]$Action = 'dump', [string]$Out = 'C:\kf\TE\ui.txt', [double]$FX = 0.1786, [double]$FY = 0.845)
$ErrorActionPreference = 'Continue'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -Namespace K -Name W -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
'@
[void][K.W]::SetProcessDPIAware()
$o = New-Object System.Collections.Generic.List[string]
function L($s) { $o.Add([string]$s) }
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, '3DMark Demo')
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if ($Action -eq 'launch' -and -not $win) {
  $lnk = Get-ChildItem 'C:\Users\Public\Desktop', "$env:USERPROFILE\Desktop", 'C:\ProgramData\Microsoft\Windows\Start Menu\Programs' -Filter '3DMark*.url' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
  L ('LAUNCH ' + $lnk.FullName); Start-Process $lnk.FullName
  for ($i = 0; $i -lt 60 -and -not $win; $i++) { Start-Sleep 1; $win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond) }
  Start-Sleep 5
}
if (-not $win -and $Action -ne 'shot') { L 'NOWINDOW'; $o | Set-Content $Out; exit 2 }
if ($win) { $h = [IntPtr]$win.Current.NativeWindowHandle; L ("WINDOW hwnd=" + $h + " rect=" + $win.Current.BoundingRectangle + " class=" + $win.Current.ClassName) } else { L 'NOWINDOW (shot only)' }
if ($Action -eq 'focus' -or $Action -eq 'run' -or $Action -eq 'click') { [void][K.W]::ShowWindow($h, 9); [void][K.W]::SetForegroundWindow($h); Start-Sleep -Milliseconds 500 }
if ($win) { $all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition); L ("descendants " + $all.Count) }
if ($Action -eq 'dump') {
  foreach ($e in $all) { $c = $e.Current; if ($c.Name -or $c.AutomationId) { L ("EL type=" + $c.ControlType.ProgrammaticName + " name='" + $c.Name + "' id=" + $c.AutomationId + " rect=" + $c.BoundingRectangle + " enabled=" + $c.IsEnabled) } }
}
if ($Action -eq 'run' -or $Action -eq 'click') {
  $r = $win.Current.BoundingRectangle
  $x = [int]($r.X + $r.Width * $FX); $y = [int]($r.Y + $r.Height * $FY)
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($x, $y); Start-Sleep -Milliseconds 400
  [K.W]::mouse_event(2, 0, 0, 0, 0); Start-Sleep -Milliseconds 80; [K.W]::mouse_event(4, 0, 0, 0, 0)
  L ("CLICKED at " + $x + "," + $y + " window=" + $r)
  L ("CLICK_AT " + (Get-Date).ToUniversalTime().ToString('o'))
  Start-Sleep -Milliseconds 1500
}
if ($Action -eq 'shot' -or $Action -eq 'run' -or $Action -eq 'click') {
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
  $bmp.Save('C:\kf\TE\shot.png', [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose(); L 'SHOT C:\kf\TE\shot.png'
}
$o | Set-Content $Out

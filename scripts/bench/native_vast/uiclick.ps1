# uiclick.ps1 [-X n -Y n] [-Shot name.png] : (interactive session) optional left click at X,Y, then a screenshot to C:\bench\<Shot>
param([int]$X = -1, [int]$Y = -1, [string]$Shot = 'ui.png', [string]$Key = '')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
'@
if ($X -ge 0) { [W.U]::SetCursorPos($X, $Y) | Out-Null; Start-Sleep -Milliseconds 300; [W.U]::mouse_event(2,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [W.U]::mouse_event(4,0,0,0,[UIntPtr]::Zero); Start-Sleep 3 }
if ($Key) { [System.Windows.Forms.SendKeys]::SendWait($Key); Start-Sleep 3 }
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bm = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bm); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); $bm.Save("C:\bench\$Shot", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bm.Dispose()

# inp.ps1 "m X Y; c; d; k VK; t text; w ms"  (move, left click, double click, key press VK hex/dec, type, wait)
param([string]$Ops,[string]$File)
if($File){$Ops=(Get-Content $File -Raw).Trim()}
Add-Type @"
using System; using System.Runtime.InteropServices;
public class I{
[DllImport("user32.dll")]public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")]public static extern bool SetCursorPos(int x,int y);
[DllImport("user32.dll")]public static extern void mouse_event(uint f,uint dx,uint dy,uint d,UIntPtr e);
[DllImport("user32.dll")]public static extern void keybd_event(byte vk,byte sc,uint f,UIntPtr e);
}
"@
[I]::SetProcessDPIAware() | Out-Null
foreach($op in $Ops -split ';'){ $a=$op.Trim() -split '\s+'
 switch($a[0]){
 'm'{[I]::SetCursorPos([int]$a[1],[int]$a[2])|Out-Null}
 'c'{[I]::mouse_event(2,0,0,0,[UIntPtr]::Zero);Start-Sleep -m 80;[I]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}
 'd'{1..2|%{[I]::mouse_event(2,0,0,0,[UIntPtr]::Zero);Start-Sleep -m 60;[I]::mouse_event(4,0,0,0,[UIntPtr]::Zero);Start-Sleep -m 80}}
 'k'{$v=[Convert]::ToByte($a[1],$(if($a[1] -like '0x*'){16}else{10}));[I]::keybd_event($v,0,0,[UIntPtr]::Zero);Start-Sleep -m 60;[I]::keybd_event($v,0,2,[UIntPtr]::Zero)}
 'w'{Start-Sleep -m ([int]$a[1])}
 }
 Start-Sleep -m 150}
"ok"

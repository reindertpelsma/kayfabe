# Wukong benchmark driver server (runs in the interactive desktop session). Commands in C:\kfhw\cmd.txt (one per line), results in C:\kfhw\; stop = C:\kfhw\stop
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms,System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public class KfW5 {
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern void keybd_event(byte vk,byte sc,uint f,UIntPtr e); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool f); [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int dx,int dy,uint d,UIntPtr e); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y); }
'@
function Shot($n){$b=[System.Windows.Forms.SystemInformation]::VirtualScreen;$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height;$gr=[System.Drawing.Graphics]::FromImage($bmp);$gr.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size);$bmp.Save("C:\kfhw\$n.png");$bmp.Dispose()}
function Focus-Pid($pidv){
 $cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$pidv)
 $w=[System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$cond) | Select -First 1
 if(-not $w){return $false}
 $h=[IntPtr]$w.Current.NativeWindowHandle
 $fgw=[KfW5]::GetForegroundWindow(); $d=0; $ft=[KfW5]::GetWindowThreadProcessId($fgw,[ref]$d); $me=[KfW5]::GetCurrentThreadId()
 [KfW5]::AttachThreadInput($me,$ft,$true) | Out-Null
 [KfW5]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [KfW5]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
 [KfW5]::SetForegroundWindow($h) | Out-Null
 [KfW5]::AttachThreadInput($me,$ft,$false) | Out-Null
 return ([KfW5]::GetForegroundWindow() -eq $h)
}
function Log($m){"$(Get-Date -f o) $m" | Add-Content C:\kfhw\srv.log}
Remove-Item C:\kfhw\stop,C:\kfhw\cmd.txt,C:\kfhw\done.txt -ErrorAction SilentlyContinue
$game='C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool'
$manifest=Get-ChildItem 'C:\Program Files (x86)\Steam\steamapps\appmanifest_*.acf' | Where-Object {(Get-Content $_.FullName -Raw) -match '"installdir"\s+"Black Myth Wukong Benchmark Tool"'} | Select-Object -First 1
$appid=[regex]::Match((Get-Content $manifest.FullName -Raw),'"appid"\s+"(\d+)"').Groups[1].Value
Log "server up appid=$appid"
$scale=1.0
while(-not (Test-Path C:\kfhw\stop)){
 if(Test-Path C:\kfhw\cmd.txt){
  $lines=Get-Content C:\kfhw\cmd.txt; Remove-Item C:\kfhw\cmd.txt
  foreach($l in $lines){
   $a=$l.Split('|'); $k=$a[0]
   $g=Get-Process -Name b1-Win64-Shipping -ErrorAction SilentlyContinue | Select -First 1
   switch($k){
    'LAUNCH' {
      Get-Process | Where-Object {$_.Name -match '^(b1_benchmark|b1-Win64-Shipping|CrashReportClient)$'} | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
      if(-not (Get-Process steam -ErrorAction SilentlyContinue)){
        Start-Process 'C:\Program Files (x86)\Steam\steam.exe' -ArgumentList '-silent' | Out-Null
        $w=(Get-Date).AddSeconds(90); while((Get-Date) -lt $w -and -not (Get-Process steamwebhelper -ErrorAction SilentlyContinue)){Start-Sleep 3}
        Start-Sleep 10 }
      $env:SteamAppId=$appid;$env:SteamGameId=$appid
      $p=Start-Process "$game\b1_benchmark.exe" -WorkingDirectory $game -PassThru
      Log "LAUNCH" }
    'SHOT' { Shot $a[1] }
    'KEY' { if($g){$f=Focus-Pid $g.Id;[System.Windows.Forms.SendKeys]::SendWait($a[1]);Log "key $($a[1]) focus=$f"}else{Log "key $($a[1]) NOGAME"} }
    'WHEEL' { if($g){Focus-Pid $g.Id|Out-Null}; $sx=[System.Windows.Forms.SystemInformation]::VirtualScreen.Width/1920.0; [KfW5]::SetCursorPos([int](960*$sx),[int](500*$sx))|Out-Null; [KfW5]::mouse_event(0x1,1,1,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 300; for($i=0;$i -lt [int]$a[1];$i++){[KfW5]::mouse_event(0x800,0,0,[uint32]4294967176,[UIntPtr]::Zero);Start-Sleep -Milliseconds 100}; Log "wheel $($a[1])" }
    'CLICK' { if($g){Focus-Pid $g.Id|Out-Null}; $sx=[System.Windows.Forms.SystemInformation]::VirtualScreen.Width/1920.0; $x=[int]([double]$a[1]*$sx);$y=[int]([double]$a[2]*$sx)
      [KfW5]::SetCursorPos($x-20,$y-10)|Out-Null; Start-Sleep -Milliseconds 200
      foreach($i in 1..10){[KfW5]::mouse_event(0x1,2,1,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 40}
      [KfW5]::SetCursorPos($x,$y)|Out-Null; [KfW5]::mouse_event(0x1,1,1,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 500
      [KfW5]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 150; [KfW5]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero); Log "click $($a[1]),$($a[2])" }
    'WAIT' { Start-Sleep ([int]$a[1]) }
    'STATUS' {
      $smi=@(& nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader,nounits 2>$null)
      $lk=@(Get-WinEvent -FilterHashtable @{LogName='Application';Id=1001;StartTime=(Get-Date).AddMinutes(-30)} -ErrorAction SilentlyContinue | Where-Object {$_.Message -match 'LiveKernelEvent'} | ForEach-Object {$_.TimeCreated.ToString('o')+' '+([regex]::Match($_.Message,'P1: (\d+)').Groups[1].Value)})
      $o=@{alive=[bool]$g;cpu=$(if($g){$g.CPU}else{0});smi=($smi -join ';');now=(Get-Date).ToString('o');lke=$lk;launcher=[bool](Get-Process b1_benchmark -ErrorAction SilentlyContinue)}
      ($o | ConvertTo-Json -Compress) | Set-Content "C:\kfhw\status_$($a[1]).json" }
   }
  }
  'done' | Set-Content C:\kfhw\done.txt
 }
 Start-Sleep -Milliseconds 500
}
Get-Process | Where-Object {$_.Name -match '^(b1_benchmark|b1-Win64-Shipping|CrashReportClient)$'} | ForEach-Object {Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue}
Log "server stop"

$ErrorActionPreference='Stop'
$root='Q4_GPU_ROOT'
if (-not ('Q4ProcessIdentity' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;
public static class Q4ProcessIdentity {
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetProcessTimes(IntPtr h,out ulong c,out ulong e,out ulong k,out ulong u);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,EntryPoint="QueryFullProcessImageNameW",SetLastError=true)] static extern bool QueryImage(IntPtr h,uint flags,StringBuilder text,ref uint size);
 public static string[] Read(IntPtr h) {
  ulong c,e,k,u;if(!GetProcessTimes(h,out c,out e,out k,out u))throw new Win32Exception(Marshal.GetLastWin32Error());
  var image=new StringBuilder(32768);uint n=32768;if(!QueryImage(h,0,image,ref n))throw new Win32Exception(Marshal.GetLastWin32Error());
  return new string[]{c.ToString(CultureInfo.InvariantCulture),image.ToString()};
 }
}
'@
}
$owned=Get-Content "$root\collectors-owned.json" -Raw|ConvertFrom-Json
$result=@()
foreach($row in $owned){
 if($row.pid -is [array]){throw 'NONSCALAR_OWNED_PID'}
 $p=Get-Process -Id $row.pid -ErrorAction SilentlyContinue;$stopped=$false
 if($p){
  $identity=[Q4ProcessIdentity]::Read($p.Handle)
  if($identity[0] -cne [string]$row.start -or -not [StringComparer]::OrdinalIgnoreCase.Equals($identity[1],[string]$row.exe)){
   @{pid=$row.pid;expected_start=[string]$row.start;actual_start=$identity[0];expected_image=[string]$row.exe;actual_image=$identity[1]}|ConvertTo-Json -Compress|Add-Content "$root\identity-rejections.jsonl"
   throw 'COLLECTOR_PID_IDENTITY_CHANGED'
  }
  $p.Kill();$p.WaitForExit();$stopped=$true
 }
 $result+=@{name=$row.name;pid=$row.pid;stopped=$stopped;errors=[IO.File]::ReadAllText("$root\$($row.name)-error.txt")}
}
$result|ConvertTo-Json -Compress -Depth 4

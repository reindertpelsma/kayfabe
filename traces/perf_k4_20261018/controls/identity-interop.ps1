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

$ErrorActionPreference='Stop'
$root='Q4_GPU_ROOT\identity-controls'
New-Item -ItemType Directory -Path $root -Force|Out-Null
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

$old=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('JEVycm9yQWN0aW9uUHJlZmVyZW5jZT0nU3RvcCcKJHJvb3Q9J19fQ09OVFJPTF9TVE9QX1JPT1RfXycKJG93bmVkPUAoR2V0LUNvbnRlbnQgIiRyb290XGNvbGxlY3RvcnMtb3duZWQuanNvbiIgLVJhd3xDb252ZXJ0RnJvbS1Kc29uKQokcmVzdWx0PUAoKQpmb3JlYWNoKCRyb3cgaW4gJG93bmVkKXskcD1HZXQtUHJvY2VzcyAtSWQgJHJvdy5waWQgLUVycm9yQWN0aW9uIFNpbGVudGx5Q29udGludWU7JHN0b3BwZWQ9JGZhbHNlO2lmKCRwKXtpZigkcC5TdGFydFRpbWUuVG9GaWxlVGltZVV0YygpIC1uZSAkcm93LnN0YXJ0IC1vciAkcC5QYXRoIC1uZSAkcm93LmV4ZSl7dGhyb3cgJ0NPTExFQ1RPUl9QSURfSURFTlRJVFlfQ0hBTkdFRCd9OyRwLktpbGwoKTskcC5XYWl0Rm9yRXhpdCgpOyRzdG9wcGVkPSR0cnVlfTskcmVzdWx0Kz1Ae25hbWU9JHJvdy5uYW1lO3BpZD0kcm93LnBpZDtzdG9wcGVkPSRzdG9wcGVkO2Vycm9ycz0oW0lPLkZpbGVdOjpSZWFkQWxsVGV4dCgiJHJvb3RcJCgkcm93Lm5hbWUpLWVycm9yLnR4dCIpKX19CiRyZXN1bHR8Q29udmVydFRvLUpzb24gLUNvbXByZXNzIC1EZXB0aCA0Cg=='))
$good=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('JEVycm9yQWN0aW9uUHJlZmVyZW5jZT0nU3RvcCcKJHJvb3Q9J19fQ09OVFJPTF9TVE9QX1JPT1RfXycKaWYgKC1ub3QgKCdRNFByb2Nlc3NJZGVudGl0eScgLWFzIFt0eXBlXSkpIHsKQWRkLVR5cGUgLVR5cGVEZWZpbml0aW9uIEAnCnVzaW5nIFN5c3RlbTsKdXNpbmcgU3lzdGVtLkNvbXBvbmVudE1vZGVsOwp1c2luZyBTeXN0ZW0uR2xvYmFsaXphdGlvbjsKdXNpbmcgU3lzdGVtLlJ1bnRpbWUuSW50ZXJvcFNlcnZpY2VzOwp1c2luZyBTeXN0ZW0uVGV4dDsKcHVibGljIHN0YXRpYyBjbGFzcyBRNFByb2Nlc3NJZGVudGl0eSB7CiBbRGxsSW1wb3J0KCJrZXJuZWwzMi5kbGwiLFNldExhc3RFcnJvcj10cnVlKV0gc3RhdGljIGV4dGVybiBib29sIEdldFByb2Nlc3NUaW1lcyhJbnRQdHIgaCxvdXQgdWxvbmcgYyxvdXQgdWxvbmcgZSxvdXQgdWxvbmcgayxvdXQgdWxvbmcgdSk7CiBbRGxsSW1wb3J0KCJrZXJuZWwzMi5kbGwiLENoYXJTZXQ9Q2hhclNldC5Vbmljb2RlLEVudHJ5UG9pbnQ9IlF1ZXJ5RnVsbFByb2Nlc3NJbWFnZU5hbWVXIixTZXRMYXN0RXJyb3I9dHJ1ZSldIHN0YXRpYyBleHRlcm4gYm9vbCBRdWVyeUltYWdlKEludFB0ciBoLHVpbnQgZmxhZ3MsU3RyaW5nQnVpbGRlciB0ZXh0LHJlZiB1aW50IHNpemUpOwogcHVibGljIHN0YXRpYyBzdHJpbmdbXSBSZWFkKEludFB0ciBoKSB7CiAgdWxvbmcgYyxlLGssdTtpZighR2V0UHJvY2Vzc1RpbWVzKGgsb3V0IGMsb3V0IGUsb3V0IGssb3V0IHUpKXRocm93IG5ldyBXaW4zMkV4Y2VwdGlvbihNYXJzaGFsLkdldExhc3RXaW4zMkVycm9yKCkpOwogIHZhciBpbWFnZT1uZXcgU3RyaW5nQnVpbGRlcigzMjc2OCk7dWludCBuPTMyNzY4O2lmKCFRdWVyeUltYWdlKGgsMCxpbWFnZSxyZWYgbikpdGhyb3cgbmV3IFdpbjMyRXhjZXB0aW9uKE1hcnNoYWwuR2V0TGFzdFdpbjMyRXJyb3IoKSk7CiAgcmV0dXJuIG5ldyBzdHJpbmdbXXtjLlRvU3RyaW5nKEN1bHR1cmVJbmZvLkludmFyaWFudEN1bHR1cmUpLGltYWdlLlRvU3RyaW5nKCl9OwogfQp9CidACn0KJG93bmVkPUdldC1Db250ZW50ICIkcm9vdFxjb2xsZWN0b3JzLW93bmVkLmpzb24iIC1SYXd8Q29udmVydEZyb20tSnNvbgokcmVzdWx0PUAoKQpmb3JlYWNoKCRyb3cgaW4gJG93bmVkKXsKIGlmKCRyb3cucGlkIC1pcyBbYXJyYXldKXt0aHJvdyAnTk9OU0NBTEFSX09XTkVEX1BJRCd9CiAkcD1HZXQtUHJvY2VzcyAtSWQgJHJvdy5waWQgLUVycm9yQWN0aW9uIFNpbGVudGx5Q29udGludWU7JHN0b3BwZWQ9JGZhbHNlCiBpZigkcCl7CiAgJGlkZW50aXR5PVtRNFByb2Nlc3NJZGVudGl0eV06OlJlYWQoJHAuSGFuZGxlKQogIGlmKCRpZGVudGl0eVswXSAtY25lIFtzdHJpbmddJHJvdy5zdGFydCAtb3IgLW5vdCBbU3RyaW5nQ29tcGFyZXJdOjpPcmRpbmFsSWdub3JlQ2FzZS5FcXVhbHMoJGlkZW50aXR5WzFdLFtzdHJpbmddJHJvdy5leGUpKXsKICAgQHtwaWQ9JHJvdy5waWQ7ZXhwZWN0ZWRfc3RhcnQ9W3N0cmluZ10kcm93LnN0YXJ0O2FjdHVhbF9zdGFydD0kaWRlbnRpdHlbMF07ZXhwZWN0ZWRfaW1hZ2U9W3N0cmluZ10kcm93LmV4ZTthY3R1YWxfaW1hZ2U9JGlkZW50aXR5WzFdfXxDb252ZXJ0VG8tSnNvbiAtQ29tcHJlc3N8QWRkLUNvbnRlbnQgIiRyb290XGlkZW50aXR5LXJlamVjdGlvbnMuanNvbmwiCiAgIHRocm93ICdDT0xMRUNUT1JfUElEX0lERU5USVRZX0NIQU5HRUQnCiAgfQogICRwLktpbGwoKTskcC5XYWl0Rm9yRXhpdCgpOyRzdG9wcGVkPSR0cnVlCiB9CiAkcmVzdWx0Kz1Ae25hbWU9JHJvdy5uYW1lO3BpZD0kcm93LnBpZDtzdG9wcGVkPSRzdG9wcGVkO2Vycm9ycz1bSU8uRmlsZV06OlJlYWRBbGxUZXh0KCIkcm9vdFwkKCRyb3cubmFtZSktZXJyb3IudHh0Iil9Cn0KJHJlc3VsdHxDb252ZXJ0VG8tSnNvbiAtQ29tcHJlc3MgLURlcHRoIDQK'))
$created=[Collections.Generic.List[object]]::new()
function Spawn-Two([string]$tag){
 $rows=@()
 foreach($i in @(0,1)){
  $p=Start-Process "$env:windir\system32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile -Command "Start-Sleep -Seconds 30"' -PassThru -WindowStyle Hidden
  $id=[Q4ProcessIdentity]::Read($p.Handle);$created.Add(@{pid=$p.Id;start=$id[0]});$name="$tag-$i"
  [IO.File]::WriteAllText("$root\$name-error.txt",'');$rows+=@{name=$name;pid=$p.Id;start=$id[0];exe=$id[1]}
 }
 return $rows
}
function Save-Rows($rows){$rows|ConvertTo-Json -Depth 4|Set-Content "$root\collectors-owned.json"}
function Invoke-Stop([string]$source){
 $stopCode=$source.Replace('__CONTROL_STOP_ROOT__',$root);[IO.File]::WriteAllText("$root\stop.ps1",$stopCode)
 try{& "$root\stop.ps1"|Out-String}catch{[string]$_.Exception.Message}
}
function Alive-Count($rows){$n=0;foreach($r in $rows){$p=Get-Process -Id $r.pid -ErrorAction SilentlyContinue;if($p -and ([Q4ProcessIdentity]::Read($p.Handle))[0] -ceq [string]$r.start){$n++}};return $n}
$result=@{classification='MEASURED';kind='sampler_identity_controls';green_failed=0;green_assertions=6;mutants=@();identity_source='GetProcessTimes + QueryFullProcessImageNameW on the process handle; creation stored as exact decimal string'}
$rows=Spawn-Two 'base';Save-Rows $rows
$badShape=@(Get-Content "$root\collectors-owned.json" -Raw|ConvertFrom-Json)
$direct=Get-Content "$root\collectors-owned.json" -Raw|ConvertFrom-Json
$result.shape=@{old_outer_count=$badShape.Count;old_first_pid_is_array=($badShape[0].pid -is [array]);direct_count=$direct.Count;direct_first_pid_is_array=($direct[0].pid -is [array]);decoded_start_type=$direct[0].start.GetType().FullName}
$result.original_stop_output=Invoke-Stop $old
$result.expected_red_positive_assertion_failures=Alive-Count $rows
if($result.expected_red_positive_assertion_failures -ne 2){throw 'ORIGINAL_DEFECT_NOT_REPRODUCED'}
$wrong=@($rows|ForEach-Object {@{name=$_.name;pid=$_.pid;start=([uint64]$_.start+1).ToString();exe=$_.exe}});Save-Rows $wrong;$null=Invoke-Stop $good
if((Alive-Count $rows) -ne 2){$result.green_failed+=2}
$wrong=@($rows|ForEach-Object {@{name=$_.name;pid=$_.pid;start=$_.start;exe=($_.exe+'.wrong')}});Save-Rows $wrong;$null=Invoke-Stop $good
if((Alive-Count $rows) -ne 2){$result.green_failed+=2}
Save-Rows $rows;$result.correct_stop_output=Invoke-Stop $good
if((Alive-Count $rows) -ne 0){$result.green_failed+=2}
foreach($kind in @('skip-starttime','skip-image')){
 $mr=Spawn-Two $kind
 if($kind -eq 'skip-starttime'){
  $wrong=@($mr|ForEach-Object {@{name=$_.name;pid=$_.pid;start=([uint64]$_.start+1).ToString();exe=$_.exe}})
  $mut=$good.Replace('$identity[0] -cne [string]$row.start -or ','')
 }else{
  $wrong=@($mr|ForEach-Object {@{name=$_.name;pid=$_.pid;start=$_.start;exe=($_.exe+'.wrong')}})
  $mut=$good.Replace(' -or -not [StringComparer]::OrdinalIgnoreCase.Equals($identity[1],[string]$row.exe)','')
 }
 Save-Rows $wrong;$null=Invoke-Stop $mut;$fail=2-(Alive-Count $mr);$result.mutants+=@{name=$kind;distinct_negative_assertion_killers=$fail}
 if($fail -ne 2){$result.green_failed++}
}
$result.mutants+=@{name='old-array-wrapper';distinct_positive_assertion_killers=$result.expected_red_positive_assertion_failures}
$result.live_own_children=@($created|Where-Object {Get-Process -Id $_.pid -ErrorAction SilentlyContinue})
$result.verdict=if($result.green_failed -eq 0 -and $result.live_own_children.Count -eq 0){'PASS'}else{'FAIL'}
$result|ConvertTo-Json -Compress -Depth 6

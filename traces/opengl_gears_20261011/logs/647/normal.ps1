$out='C:\kf\normal-result.txt'
Set-Content $out ('START '+[DateTime]::UtcNow.ToString('o'))
foreach($t in 'cup2','cup3','cup8') {
 & ('D:\tools\'+$t+'.exe') *> ('C:\kf\'+$t+'-normal.txt')
 Add-Content $out ($t+' EXIT '+$LASTEXITCODE)
}
& 'D:\tools\kf_glgears.exe' --seconds 45 *> 'C:\kf\gears-normal.txt'
Add-Content $out ('GEARS EXIT '+$LASTEXITCODE)
Get-CimInstance Win32_PnPEntity | Where-Object Name -Like '*NVIDIA*' | ForEach-Object {Add-Content $out ('ADAPTER '+$_.Name+' CODE '+$_.ConfigManagerErrorCode)}
Add-Content $out ('END '+[DateTime]::UtcNow.ToString('o'))

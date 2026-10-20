$ErrorActionPreference='Stop'
$r='C:\ProgramData\KayfabeSweepNative'
$g=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
if($g.Count -ne 1 -or $g[0].DriverVersion -ne '32.0.15.8088' -or $g[0].ConfigManagerErrorCode -ne 0){throw 'Native reference must have actual8088/Code0'}
$g | Select-Object Name,DriverVersion,ConfigManagerErrorCode | ConvertTo-Json -Compress
foreach($row in @(@('cup2.exe','86039b888dc7d17b1bd92736c5a5589a4a67a897a5aa9859f7a7ed9d280d95b8','CE rv=0xabcd1234 want=0xabcd1234 -> PASS'),@('cup3.exe','41681173e42c40ba94cafd33be22c90f419b64436b9be88993ba6bd286a19460','KERNEL rv=43 want=43 -> PASS'),@('cup8.exe','7401a67a58288e0789a8fa214be4696316715c5306b690781996847f5baffcf6','CUP8 RESULT N=2048 bad=0 maxerr=0 '))){
 $file="$r\build\$($row[0])"
 if((Get-FileHash $file).Hash.ToLower() -ne $row[1]){throw 'Native probe hash mismatch'}
 $output=@(& $file 2>&1);$rc=$LASTEXITCODE;$output | ForEach-Object {Write-Output $_}
 if($rc -ne 0 -or !(($output -join "`n").Contains($row[2]))){throw "Native probe $($row[0]) failed values or rc=$rc"}
}
$smi="$env:SystemRoot\System32\nvidia-smi.exe"
if(!(Test-Path $smi)){$smi="$env:ProgramFiles\NVIDIA Corporation\NVSMI\nvidia-smi.exe"}
& $smi -q | Select-String 'GSP Firmware Version'
'NATIVE_CUDA3_PASS actual8088_Code0=1'

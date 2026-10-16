$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
pnputil.exe /scan-devices
$drives=@([IO.DriveInfo]::GetDrives())
foreach($drive in $drives){ Write-Output "DRIVE root=$($drive.RootDirectory.FullName) type=$($drive.DriveType) ready=$($drive.IsReady)"; if($drive.IsReady -and $drive.DriveType -eq [IO.DriveType]::CDRom){Get-ChildItem $drive.RootDirectory.FullName | Select-Object Name,Length | ConvertTo-Json -Compress} }

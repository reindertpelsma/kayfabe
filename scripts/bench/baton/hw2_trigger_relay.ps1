param([int]$Port=39021)
$ErrorActionPreference='Stop'
$udp=New-Object Net.Sockets.UdpClient
try {
    $b=[Text.Encoding]::UTF8.GetBytes('HW2_RELAY_READY')
    $null=$udp.Send($b,$b.Length,'10.0.2.2',$Port)
    Get-Content 'C:\kf\hang\trig.log' -Tail 0 -Wait | ForEach-Object {
        $b=[Text.Encoding]::UTF8.GetBytes([string]$_)
        if($b.Length -le 4096) { $null=$udp.Send($b,$b.Length,'10.0.2.2',$Port) }
    }
} finally { $udp.Dispose() }

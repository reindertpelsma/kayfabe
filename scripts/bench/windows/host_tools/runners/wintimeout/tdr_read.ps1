$p = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers'
"TdrDelay=" + $p.TdrDelay + " TdrDdiDelay=" + $p.TdrDdiDelay + " TdrLevel=" + $p.TdrLevel + " HwSchMode=" + $p.HwSchMode

#!/usr/bin/env python3
"""Confirm the root cause on the LTSC baseline overlay: first start without EnableGpuFirmware -> Code 43;
then set EnableGpuFirmware=1 (service key + the device's class key) and restart -> expect Code 0 and kayfabe Running. usage: RUN OUT"""
import sys, time, re
sys.path.insert(0, "/data/kf-prod-defaults-host/scripts/bench/windows/appmatrix")
from qga import Qga

run, out = sys.argv[1:3]
PS = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"


def log(m):
    open(out, "a").write(time.strftime("%H:%M:%S ") + m + "\n")
    print(m, flush=True)


def wait(t=300):
    t0 = time.time()
    while time.time() - t0 < t:
        try:
            q = Qga(run + "/qga.sock")
            if q.ping():
                return q
        except Exception:
            pass
        time.sleep(3)
    return None


def ps(q, s, timeout=120):
    r = q.exec(PS, ["-NoProfile", "-Command", s], timeout=timeout)
    return (r.out.decode(errors="replace") if isinstance(r.out, bytes) else str(r.out)).strip()


def phase():
    try:
        l = [x for x in open(run + "/qemu.log", errors="ignore") if "kf3: family=" in x][-1]
        return re.search(r"phase=(\w+) trapped=(\d+)", l).group(0)
    except Exception:
        return "?"


def reboot(q):
    ps(q, "shutdown /r /t 0", 20)
    q.close()
    time.sleep(30)
    return wait(300)


GPU = r"(Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + ' code=' + $_.ConfigManagerErrorCode }) -join '; '"
q = wait()
log("guest up: " + ps(q, GPU) + " | " + phase())
log("EnableGpuFirmware before: " + ps(q, r"""
$k = (Get-PnpDeviceProperty -InstanceId (Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_2786*' }).InstanceId -KeyName DEVPKEY_Device_Driver).Data
"classkey=$k svc=" + (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -ErrorAction SilentlyContinue).EnableGpuFirmware + " class=" + (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$k" -ErrorAction SilentlyContinue).EnableGpuFirmware
""").replace("\n", " "))
ps(q, r"""
Remove-Item -Recurse -Force 'C:\ProgramData\NVIDIA Corporation' -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force 'HKLM:\SOFTWARE\NVIDIA Corporation' -ErrorAction SilentlyContinue
$id = (Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_2786*' }).InstanceId
pnputil /remove-device "$id" | Out-Null
Remove-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -Name EnableGpuFirmware -ErrorAction SilentlyContinue
""", 300)
q.close()
time.sleep(10)
q = wait(120)
q = reboot(q)
log("after remove + reboot (flag absent): " + (ps(q, GPU) if q else "no agent") + " | " + phase())
log("flag now: " + ps(q, r"""
$k = (Get-PnpDeviceProperty -InstanceId (Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_2786*' }).InstanceId -KeyName DEVPKEY_Device_Driver).Data
"classkey=$k svc=" + (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -ErrorAction SilentlyContinue).EnableGpuFirmware + " class=" + (Get-ItemProperty "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$k" -ErrorAction SilentlyContinue).EnableGpuFirmware
""").replace("\n", " "))
log("set flag: " + ps(q, r"""
$id = (Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_2786*' }).InstanceId
$k = (Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_Driver).Data
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\Class\$k" -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm' -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null
"set class $k and service key"
""").replace("\n", " "))
q = reboot(q)
log("after setting EnableGpuFirmware=1 + reboot: " + (ps(q, GPU) if q else "no agent") + " | " + phase())
if q:
    q.close()

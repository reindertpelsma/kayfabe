#!/usr/bin/env python3
"""First-start test v2 on the LTSC baseline overlay: keep the driver package in the DriverStore, delete NVIDIA state and the PCI
device node, reboot (Windows re-detects the GPU as new hardware and binds the stored driver). usage: freshdrv2.py RUN OUT"""
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
if not q:
    log("no guest agent")
    sys.exit(2)
log("guest up: " + ps(q, GPU) + " | " + phase())
rem = ps(q, r"""
Remove-Item -Recurse -Force 'C:\ProgramData\NVIDIA Corporation' -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force 'HKLM:\SOFTWARE\NVIDIA Corporation' -ErrorAction SilentlyContinue
$id = (Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&DEV_2786*' }).InstanceId
"device: $id"
pnputil /remove-device "$id" | Out-String
""", 300)
log("remove-device: " + rem.replace("\n", " ")[:300])
q.close()
time.sleep(10)
q = wait(120)
log("after remove-device (no reboot): " + (ps(q, GPU) if q else "no agent") + " | " + phase())
if q:
    q = reboot(q)
log("after reboot 1 (re-detected as new hardware): " + (ps(q, GPU) if q else "no agent") + " | " + phase())
time.sleep(60)
if q:
    q.close()
    q = wait(60)
log("60 s later: " + (ps(q, GPU) if q else "no agent") + " | " + phase())
if q:
    q = reboot(q)
log("after reboot 2: " + (ps(q, GPU) if q else "no agent") + " | " + phase())
if q:
    q.close()

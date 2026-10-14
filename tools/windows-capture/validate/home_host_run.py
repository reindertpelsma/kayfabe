#!/usr/bin/env python3
"""kfwc validation driver (trusted host). usage: claude-agent-kfwc.py PHASE RUN OUTDIR
PHASE A: boot 1 of the overlay: Preflight, testsigning on, binaries into C:\\kfwc.
PHASE B: boot 2 (observer on): sign in, Install, capture with a forced driver restart, scan-cost windows.
Logs to OUTDIR/phase<X>.txt. Never prints the guest password."""
import sys, time, re, os, subprocess, base64, json
sys.path.insert(0, "/data/kf-prod-defaults-host/scripts/bench/windows/appmatrix")
from qga import Qga, QgaError

phase, run, outdir = sys.argv[1:4]
PS = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
PW = open('/var/lib/kf-windows-20261005/windows-desktop/secrets/win_password').read().strip()
LOG = os.path.join(outdir, "phase%s.txt" % phase)


def log(m):
    with open(LOG, "a") as f:
        f.write(time.strftime("%H:%M:%S ") + m + "\n")
    print(m, flush=True)


def wait(t=420):
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


def ps(q, s, timeout=90):
    r = None
    for attempt in range(4):
        try:
            r = q.exec(PS, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", s], timeout=timeout)
            break
        except QgaError as e:
            log("   qga error %r (retry %d)" % (e, attempt))
            q.close(); time.sleep(10)
    if r is None:
        return "[QGA ERROR]"
    o = r.out.decode(errors="replace") if isinstance(r.out, bytes) else str(r.out)
    e = r.err.decode(errors="replace") if isinstance(r.err, bytes) and r.err else ""
    if r.timed_out:
        o += "\n[TIMED OUT]"
    return (o + (("\n[stderr] " + e) if e.strip() else "")).strip()


def fetch(q, gpath, local):
    h = q.call("guest-file-open", {"path": gpath, "mode": "rb"})
    n = 0
    with open(local, "wb") as f:
        while True:
            r = q.call("guest-file-read", {"handle": h, "count": 1 << 20})
            b = base64.b64decode(r.get("buf-b64", "") or "")
            f.write(b)
            n += len(b)
            if r.get("eof") or not r.get("count"):
                break
    q.call("guest-file-close", {"handle": h})
    return n


def qemu_resets():
    try:
        return sum(1 for x in open(run + "/qemu.log", errors="ignore") if "GSP phase Running -> Suspending" in x)
    except Exception:
        return -1


def signin():
    for k in range(4):
        rc = subprocess.run([sys.executable, '/data/kf-prod-defaults-host/scripts/bench/windows/signin.py', run + '/qmp.sock', run + '/qga.sock'],
                            env=dict(os.environ, KF_GUEST_PW=PW), capture_output=True, text=True, timeout=240).returncode
        if rc == 0:
            return True
        time.sleep(15)
    return False


D = "/var/lib/kf-windows-20261005/claude-agent-kfwc"
GPU = r"(Get-CimInstance Win32_VideoController | Where-Object {$_.Name -match 'NVIDIA'} | ForEach-Object { 'code=' + $_.ConfigManagerErrorCode + ' ' + $_.Name + ' ' + $_.DriverVersion }) -join ';'"

if phase == "A":
    q = wait()
    if not q:
        log("FAIL no guest agent"); sys.exit(2)
    log("guest agent up")
    ps(q, r"New-Item -ItemType Directory -Force C:\kfwc | Out-Null")
    for fn in ("kfwc.sys", "kfwc_collect.exe", "SHA256SUMS", "install.ps1"):
        data = open(os.path.join(D, fn), "rb").read()
        q.file_write("C:\\kfwc\\" + fn, data)
        log("wrote %s %d bytes" % (fn, len(data)))
    log("hashes in guest:\n" + ps(q, r"Get-FileHash -Algorithm SHA256 C:\kfwc\kfwc.sys,C:\kfwc\kfwc_collect.exe | ForEach-Object { $_.Hash.ToLower() + '  ' + (Split-Path $_.Path -Leaf) }"))
    log("expected:\n" + open(os.path.join(D, "SHA256SUMS")).read())
    log("PREFLIGHT:\n" + ps(q, r"& C:\kfwc\install.ps1 -Action Preflight"))
    log("secureboot (Confirm-SecureBootUEFI): " + ps(q, r"try { Confirm-SecureBootUEFI } catch { 'n/a: ' + $_.Exception.Message }"))
    log("ENABLE testsigning:\n" + ps(q, r"& C:\kfwc\install.ps1 -Action EnableTestSigning"))
    log("bcdedit now:\n" + ps(q, r"(bcdedit /enum '{current}') -join ' | '"))
    log("gpu: " + ps(q, GPU))
    q.close()
    sys.exit(0)

if phase == "C":
    # one boot (testsigning is already on in the baseline): install, scan-only capture across a driver restart,
    # then the back-off reset test: device disabled, collector with the default back-off, device enabled late.
    q = wait()
    if not q:
        log("FAIL no guest agent"); sys.exit(2)
    log("guest agent up; observer file exists: %s" % os.path.exists(run + "/gsp.jsonl"))
    ps(q, r"New-Item -ItemType Directory -Force C:\kfwc | Out-Null")
    for fn in ("kfwc.sys", "kfwc_collect.exe", "SHA256SUMS", "install.ps1"):
        q.file_write("C:\\kfwc\\" + fn, open(os.path.join(D, fn), "rb").read())
    log("INSTALL:\n" + ps(q, r"& C:\kfwc\install.ps1 -Action Install -Build C:\kfwc", 180)[-700:])
    if "RUNNING" not in ps(q, r"sc.exe query KayfabeWinCapture"):
        log("DRIVER NOT RUNNING"); sys.exit(3)
    iid = ps(q, r"(Get-PnpDevice -Class Display | Where-Object {$_.FriendlyName -match 'NVIDIA'}).InstanceId").strip()

    def start_collector(name, args):
        ps(q, r"Remove-Item -Recurse -Force C:\kfwc\%s -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force C:\kfwc\%s | Out-Null" % (name, name))
        cmd = r"cmd /c C:\kfwc\kfwc_collect.exe C:\kfwc\%s %s > C:\kfwc\%s.out 2>&1" % (name, args, name)
        return ps(q, r"$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine='%s'}; 'create rc ' + $r.ReturnValue" % cmd)

    def wait_gone(maxs):
        t0 = time.time()
        while time.time() - t0 < maxs:
            if "gone" in ps(q, r"if (Get-Process kfwc_collect -ErrorAction SilentlyContinue) { 'running' } else { 'gone' }"):
                return True
            time.sleep(5)
        return False

    def grab(name):
        log("%s out: " % name + ps(q, r"Get-Content C:\kfwc\%s.out -ErrorAction SilentlyContinue | Select-Object -First 8 | Out-String" % name).replace("\n", " | ")[:700])
        for fn in ("capture.kfgspcap", "cell.json"):
            try:
                n = fetch(q, "C:\\kfwc\\%s\\%s" % (name, fn), os.path.join(outdir, "%s_%s" % (name, fn)))
                log("fetched %s %s %d bytes" % (name, fn, n))
            except Exception as e:
                log("fetch failed %s %s: %r" % (name, fn, e))

    # ---- D: scan only (no mailbox), live queues, forced restart in the middle
    log("START D (scan only): " + start_collector("capD", "--seconds 140 --no-mailbox --decode-tag 580.82.07"))
    time.sleep(70)
    log("D: FORCED RESTART: " + ps(q, r"pnputil.exe /restart-device '%s' | Select-Object -Last 1" % iid, 120))
    time.sleep(10)
    log("D: gpu=" + ps(q, GPU) + " resets=%d" % qemu_resets())
    log("D: gone=%s" % wait_gone(120))
    grab("capD")

    # ---- E: back-off reset. Device disabled -> nothing to find; passes with growing pauses; device enabled late.
    ps(q, r"Disable-PnpDevice -InstanceId '%s' -Confirm:$false -ErrorAction SilentlyContinue; 'disabled'" % iid, 120)
    time.sleep(10)
    log("E: gpu after disable: " + ps(q, GPU))
    log("START E (default back-off 5 s doubling to 300 s): " + start_collector("capE", "--seconds 200 --decode-tag 580.82.07"))
    t_e = time.time()
    prev = cpu_sys(q)
    while time.time() - t_e < 150:
        time.sleep(8)
        cur = cpu_sys(q)
        (ta, _), (tb, _) = prev.split(), cur.split()
        log("E t+%3ds System-process CPU over the last interval: %.2f s" % (time.time() - t_e, float(tb) - float(ta)))
        prev = cur
    sw0 = ps(q, r"[System.Diagnostics.Stopwatch]::GetTimestamp()")
    en = ps(q, r"Enable-PnpDevice -InstanceId '%s' -Confirm:$false -ErrorAction SilentlyContinue; [System.Diagnostics.Stopwatch]::GetTimestamp()" % iid, 120)
    log("E: guest QPC just before enable %s, after enable %s" % (sw0.strip(), en.strip()))
    log("E: gone=%s" % wait_gone(120))
    grab("capE")
    log("gpu at end: " + ps(q, GPU))
    log("PHASE C DONE resets=%d" % qemu_resets())
    q.close()
    sys.exit(0)

# ------------------------------------------------------------------------------------ phase B
def cpu_sys(q):
    return ps(q, r"$p=Get-Process -Id 4; [string]$p.TotalProcessorTime.TotalSeconds + ' ' + [string](Get-Date).ToUniversalTime().ToString('o')")


def window_cpu(q, seconds):
    a = cpu_sys(q); time.sleep(seconds); b = cpu_sys(q)
    ta, da = a.split(); tb, db = b.split()
    return float(tb) - float(ta), seconds


q = wait()
if not q:
    log("FAIL no guest agent"); sys.exit(2)
log("guest agent up; observer file exists: %s" % os.path.exists(run + "/gsp.jsonl"))
q.close()
log("signed in: %s ; resets so far %d" % (signin(), qemu_resets()))
q = wait(60)
log("bcdedit: " + ps(q, r"(bcdedit /enum '{current}') -join ' | '")[:300])
log("gpu: " + ps(q, GPU))
log("nvidia-smi: " + ps(q, r"& 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=name,driver_version --format=csv,noheader 2>&1"))
log("INSTALL:\n" + ps(q, r"& C:\kfwc\install.ps1 -Action Install -Build C:\kfwc", 120))
log("sc query:\n" + ps(q, r"sc.exe query KayfabeWinCapture"))
log("codeintegrity (last 5): " + ps(q, r"Get-WinEvent -LogName 'Microsoft-Windows-CodeIntegrity/Operational' -MaxEvents 5 -ErrorAction SilentlyContinue | ForEach-Object { [string]$_.Id + ' ' + $_.Message.Split([char]10)[0] }"))
log("status: " + ps(q, r"& C:\kfwc\kfwc_collect.exe --status 2>&1"))
if "RUNNING" not in ps(q, r"sc.exe query KayfabeWinCapture"):
    log("DRIVER NOT RUNNING; stop"); sys.exit(3)

# --- idle baseline for the System process CPU (module loaded, collector not running)
cpu0, secs = window_cpu(q, 30)
log("BASELINE System-process CPU over %ds with the driver loaded and idle: %.3f s" % (secs, cpu0))

# --- capture A: live queues, normal work, forced driver restart
iid = ps(q, r"(Get-PnpDevice -Class Display | Where-Object {$_.FriendlyName -match 'NVIDIA'}).InstanceId")
log("nvidia instance id present: %s" % bool(iid))
ps(q, r"Remove-Item -Recurse -Force C:\kfwc\capA -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force C:\kfwc\capA | Out-Null")
t_cap = time.time()
r0 = qemu_resets()
log("START collector A: " + ps(q, r"$p = Start-Process -FilePath C:\kfwc\kfwc_collect.exe -ArgumentList 'C:\kfwc\capA','--seconds','150','--decode-tag','580.82.07' -WindowStyle Hidden -PassThru -RedirectStandardOutput C:\kfwc\capA.out -RedirectStandardError C:\kfwc\capA.err; 'pid ' + $p.Id"))
time.sleep(25)
log("t+25 status: " + ps(q, r"& C:\kfwc\kfwc_collect.exe --status 2>&1")[:400])
for i in range(3):
    log("normal work nvidia-smi: " + ps(q, r"& 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=utilization.gpu,clocks.gr,temperature.gpu --format=csv,noheader 2>&1"))
    time.sleep(5)
time.sleep(10)
log("FORCED RESTART pnputil /restart-device (resets before %d): " % qemu_resets() + ps(q, r"pnputil.exe /restart-device '%s'" % iid.strip(), 120))
t_restart = time.time()
time.sleep(25)
log("after restart: gpu=" + ps(q, GPU) + " resets now %d" % qemu_resets())
for i in range(3):
    log("normal work nvidia-smi: " + ps(q, r"& 'C:\Windows\System32\nvidia-smi.exe' --query-gpu=utilization.gpu,clocks.gr --format=csv,noheader 2>&1"))
    time.sleep(5)
# wait for the collector to finish
for i in range(60):
    if "gone" in ps(q, r"if (Get-Process kfwc_collect -ErrorAction SilentlyContinue) { 'running' } else { 'gone' }"):
        break
    time.sleep(5)
log("collector A out: " + ps(q, r"Get-Content C:\kfwc\capA.out, C:\kfwc\capA.err -ErrorAction SilentlyContinue | Select-Object -First 20 | Out-String").replace("\n", " | ")[:1500])
log("capA files: " + ps(q, r"Get-ChildItem C:\kfwc\capA | ForEach-Object { $_.Name + ' ' + $_.Length }").replace("\n", " ; "))
for fn in ("capture.kfgspcap", "cell.json"):
    try:
        n = fetch(q, "C:\\kfwc\\capA\\" + fn, os.path.join(outdir, "capA_" + fn))
        log("fetched capA %s %d bytes" % (fn, n))
    except Exception as e:
        log("fetch failed %s: %r" % (fn, e))

# --- scan cost: NVIDIA device disabled (no queues), collector with the default back-off, then without
ps(q, r"Disable-PnpDevice -InstanceId '%s' -Confirm:$false -ErrorAction SilentlyContinue; 'disabled'" % iid.strip(), 120)
time.sleep(10)
log("gpu after disable: " + ps(q, GPU))
for name, extra, secs in (("capB", ["--seconds", "170"], 170), ("capC", ["--seconds", "60", "--no-backoff"], 60)):
    ps(q, r"Remove-Item -Recurse -Force C:\kfwc\%s -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force C:\kfwc\%s | Out-Null" % (name, name))
    a = cpu_sys(q)
    ps(q, r"$p = Start-Process -FilePath C:\kfwc\kfwc_collect.exe -ArgumentList 'C:\kfwc\%s',%s,'--decode-tag','580.82.07' -WindowStyle Hidden -PassThru -RedirectStandardOutput C:\kfwc\%s.out -RedirectStandardError C:\kfwc\%s.err; $p.Id" %
       (name, ",".join("'%s'" % x for x in extra), name, name))
    t0 = time.time()
    while time.time() - t0 < secs + 40:
        time.sleep(10)
        if "gone" in ps(q, r"if (Get-Process kfwc_collect -ErrorAction SilentlyContinue) { 'running' } else { 'gone' }"):
            break
    b = cpu_sys(q)
    ta, da = a.split(); tb, db = b.split()
    log("%s: System-process CPU delta %.3f s over %.0f s wall (baseline idle rate %.4f s/s)" % (name, float(tb) - float(ta), time.time() - t0, cpu0 / 30.0))
    log("%s out: " % name + ps(q, r"Get-Content C:\kfwc\%s.out, C:\kfwc\%s.err -ErrorAction SilentlyContinue | Select-Object -First 12 | Out-String" % (name, name)).replace("\n", " | ")[:900])
    for fn in ("capture.kfgspcap", "cell.json"):
        try:
            n = fetch(q, "C:\\kfwc\\%s\\%s" % (name, fn), os.path.join(outdir, "%s_%s" % (name, fn)))
            log("fetched %s %s %d bytes" % (name, fn, n))
        except Exception as e:
            log("fetch failed %s %s: %r" % (name, fn, e))
ps(q, r"Enable-PnpDevice -InstanceId '%s' -Confirm:$false -ErrorAction SilentlyContinue; 'enabled'" % iid.strip(), 120)
time.sleep(15)
log("gpu after enable: " + ps(q, GPU))
log("PHASE B DONE resets=%d" % qemu_resets())
q.close()

#!/usr/bin/env python3
"""One OFFLINE throwaway clone of a sealed Win11-base image, checked through the repo's own pipeline (propimage_check.py --apps none), with the
guest held after the verify row so extra checks run over the image's own ssh key: network negative test, Device Manager problem code, GPU fields
against the host's, registry readback, clock window, event-log errors. Then the kf3 census and the host dmesg delta. Everything lands in
PROP_DIR/evidence/<tag>/ (text only). The work dir (a thin overlay) is deleted at the end; the sealed image is never written.
usage: w_clone_check.py TAG VARIANT SEALED_ID      VARIANT: baseline | no-guest-driver | uuid-auto   (the ablations change only the kf3 device properties)
Runs ON the pc as root. Takes /root/prov/pc_gpu.lock (bounded blocking wait), START/END lines in /root/prov/PC_ACTIVE.log.
"""
import fcntl
import json
import os
import re
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401
import w_gpu  # noqa: E402  (REG_PS, host_cuda_ready)

CK = load_check()
PI = load_pi()
LOCK, ACTIVE = "/root/prov/pc_gpu.lock", "/root/prov/PC_ACTIVE.log"
BROKER = os.environ.get("PROP_BROKER", w_gpu.BROKER)

EXTRA_PS = r"""
$ErrorActionPreference='SilentlyContinue'
$o=[ordered]@{}
$o.guest_utc=(Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$o.uptime_s=[int]((Get-Date)-(Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalSeconds
$o.tcp_1_1_1_1_443=$null; try { $o.tcp_1_1_1_1_443=[bool](Test-NetConnection -ComputerName 1.1.1.1 -Port 443 -WarningAction SilentlyContinue).TcpTestSucceeded } catch { $o.tcp_1_1_1_1_443='error' }
try { Resolve-DnsName example.com -Type A -DnsOnly -ErrorAction Stop | Out-Null; $o.dns='RESOLVED' } catch { $o.dns='FAILED' }
try { $r=Invoke-WebRequest -Uri 'http://example.com' -UseBasicParsing -TimeoutSec 8; $o.http=[string]$r.StatusCode } catch { $o.http='FAILED' }
$o.default_route=@(Get-NetRoute -DestinationPrefix '0.0.0.0/0' | ForEach-Object { "$($_.NextHop) metric $($_.RouteMetric)" })
$d=Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' } | Select-Object -First 1
$o.pnp=[ordered]@{name=$d.FriendlyName;status=[string]$d.Status;problem=[string]$d.Problem;code=(Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_ProblemCode).Data;driver_version=(Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_DriverVersion).Data}
$o.problem_devices=@(Get-PnpDevice -PresentOnly | Where-Object { $_.Status -ne 'OK' } | ForEach-Object { "$($_.Class)|$($_.FriendlyName)|problem=$($_.Problem)" })
$o.smi_query=@(& nvidia-smi --query-gpu=name,driver_version,memory.total,memory.used,temperature.gpu,fan.speed,power.draw,power.limit,clocks.sm,clocks.mem,utilization.gpu,utilization.memory,pstate,pcie.link.gen.current --format=csv 2>&1)
$o.smi_names_uuid_count=@(& nvidia-smi --query-gpu=uuid --format=csv,noheader 2>&1).Count
$o.events_since_boot=@(Get-WinEvent -FilterHashtable @{LogName='System';Level=1,2;StartTime=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime} -ErrorAction SilentlyContinue | Select-Object -First 25 | ForEach-Object { "$($_.Id) $($_.ProviderName)" })
$o.display_events=@(Get-WinEvent -FilterHashtable @{LogName='System';ProviderName='Display','nvlddmkm'} -MaxEvents 10 -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Id) $($_.ProviderName)" })
$o.kernel_power_41=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=41;StartTime=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime} -ErrorAction SilentlyContinue).Count
$o.crash_dump_files=@(Get-ChildItem C:\Windows\Minidump -ErrorAction SilentlyContinue).Count
$o | ConvertTo-Json -Compress -Depth 5
"""

HOST_Q = "name,driver_version,memory.total,memory.used,temperature.gpu,fan.speed,power.draw,power.limit,clocks.sm,clocks.mem,utilization.gpu,utilization.memory,pstate,pcie.link.gen.current"


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def active(msg):
    with open(ACTIVE, "a") as f:
        f.write("%s win11base %s\n" % (now(), msg))


def scrub(t):
    t = re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", t)
    return t


def main():
    tag, variant, sid = sys.argv[1:4]
    sealed = D / "sealed" / sid
    assert (sealed / "manifest.json").is_file(), sealed
    out = D / "evidence" / tag
    work = D / "runs" / tag
    assert not out.exists() and not work.exists(), tag
    out.mkdir(parents=True)
    release = D / "run" / ("release_" + tag)
    release.unlink(missing_ok=True)
    fd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    t0 = time.time()
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except OSError:
            if time.time() - t0 > 3600:
                print("lock busy 1 h: giving up", flush=True)
                return 7
            time.sleep(20)
    active("START kf3 offline clone check tag=%s variant=%s (restrict=on, recorded clock, headless test broker, throwaway overlay)" % (tag, variant))
    rc = 1
    try:
        if sh(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"], check=False).returncode != 0:
            print("host nvidia-smi does not answer: not starting kf3", flush=True)
            return 9
        w_gpu.OUT = out
        if not w_gpu.host_cuda_ready():
            print("host cuInit not ready: not starting kf3", flush=True)
            return 8
        need_free(15, "clone overlay")
        dmesg0 = sh(["dmesg"]).stdout.count("\n")
        cmd = ["python3", "-B", "-I", str(D / "bin/w_check_variant.py"), variant, "--", "--dir", str(sealed), "--work", str(work), "--out", str(out / "h"),
               "--qemu", KF_QEMU, "--flags", str(D / "bin/kf3_flags.sample.env"), "--broker", BROKER, "--gop-efi", str(D / "firmware/kf-gop.signed.efi"),
               "--apps", "none", "--hold-until-file", str(release)]
        (out / "cmd.txt").write_text(" ".join(cmd) + "\n")
        so = open(out / "stdout.txt", "w")
        import subprocess
        env = dict(os.environ)
        p = subprocess.Popen(["nice", "-n", "10"] + cmd, stdout=so, stderr=subprocess.STDOUT, env=env, stdin=subprocess.DEVNULL)
        holding = False
        t1 = time.time()
        while p.poll() is None and time.time() - t1 < 1500:
            txt = (out / "stdout.txt").read_text(errors="replace")
            if re.search(r"^verify (PASS|FAIL)", txt, re.M) or re.search(r"^gpu FAIL", txt, re.M) or re.search(r"^power NOT OK", txt, re.M):
                holding = True
                break
            time.sleep(5)
        extra = {}
        if holding and p.poll() is None:
            st = json.loads((sealed / "state.json").read_text())
            key = D / "run" / ("id_clone_" + tag)
            key.write_text(st["guest"]["ssh_private_key"])
            key.chmod(0o600)
            try:
                g = CK.Guest(st, str(key), st["ids"]["net"]["ssh_port"])
                r = g.ps(EXTRA_PS, 300)
                line = [l for l in r.stdout.splitlines() if l.startswith("{")]
                extra = json.loads(line[-1]) if line else {"error": (r.stdout + r.stderr)[-500:]}
                reg = g.ps(w_gpu.REG_PS, 120)
                (out / "registry.json").write_text(reg.stdout)
                smi = g.ps("nvidia-smi", 150)
                (out / "nvidia_smi.txt").write_text(scrub(smi.stdout))
                host = sh(["nvidia-smi", "--query-gpu=" + HOST_Q, "--format=csv"], check=False)
                (out / "host_nvidia_smi_query.txt").write_text(host.stdout)
                # the throwaway clone's own recorded clock window (the harness rewrote work/state.json with the adjusted stop time)
                wst = json.loads((work / "state.json").read_text()) if (work / "state.json").is_file() else {}
                if wst:
                    base = PI.utc(wst["clock"]["last_time_at_stop_utc"]) + __import__("datetime").timedelta(seconds=wst["clock"]["gap_s"])
                    extra["rtc_base_utc"] = PI.fmt(base)
                    gu = PI.utc(extra["guest_utc"]) if extra.get("guest_utc") else None
                    if gu:
                        extra["clock_offset_s"] = int((gu - base).total_seconds())
                        extra["clock_in_window"] = 0 <= extra["clock_offset_s"] <= int(extra.get("uptime_s") or 0) + 300
                    extra["stop_time_in_state"] = st["clock"]["last_time_at_stop_utc"]
                    extra["guest_after_stop_time"] = bool(gu and gu >= PI.utc(st["clock"]["last_time_at_stop_utc"]))
            finally:
                key.unlink(missing_ok=True)
        (out / "extra.json").write_text(scrub(json.dumps(extra, indent=1)) + "\n")
        release.write_text("release\n")
        try:
            rc = p.wait(timeout=900)
        except subprocess.TimeoutExpired:
            p.terminate()
            rc = "TIMEOUT"
        so.close()
        # host side evidence
        sys.path.insert(0, str(D / "bin"))
        import kf3_census
        ql = work / "run/qemu.log"
        if ql.exists():
            lines = [l.decode("utf-8", "replace") for l in open(ql, "rb")]
            c = kf3_census.census(lines)
            (out / "census.json").write_text(json.dumps(c, indent=1))
            (out / "qemu_log_tail.txt").write_text("\n".join(kf3_census.clean(l.rstrip("\n")) for l in lines[-120:]) + "\n")
            wl = [kf3_census.clean(l.rstrip("\n")) for l in lines if re.search(r"WINDOWS PROFILE|FABRICATED answer|scheduled_at_birth|gr_tier", l)]
            (out / "windows_profile_lines.txt").write_text("\n".join(wl[:60]) + "\n")
            (out / "qemu_log_lines.txt").write_text("%d lines, %d windows-profile lines\n" % (len(lines), len(wl)))
        lc = work / "run/last_cmd.txt"
        if lc.exists():
            raw = lc.read_text()
            m = re.sub(r"(-uuid )\S+", r"\1<id>", raw)
            m = re.sub(r"serial=[^,\s]+", "serial=<id>", m)
            m = re.sub(r"mac=[^,\s]+", "mac=<id>", m)
            m = re.sub(r"gpu-uuid=[^,\s]+", "gpu-uuid=<id>", m)
            m = re.sub(r"net=\d+\.\d+\.\d+\.\d+/\d+", "net=<subnet>", m)
            m = re.sub(r"hostfwd=tcp:127\.0\.0\.1:\d+", "hostfwd=tcp:127.0.0.1:<port>", m)
            m = re.sub(r"base=\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d", "base=<recorded-clock>", m)
            m = re.sub(r"(/workspace/\S*?)(runs/\S+?)(/|\s)", r"\1<work>\3", m)
            (out / "argv_masked.txt").write_text(m)
        d = sh(["dmesg"]).stdout.splitlines()[dmesg0:]
        keep = [l for l in d if re.search(r"NVRM|Xid|kf3|nvidia|DMAR|AMD-Vi|IOMMU|AER|BUG:|Oops|oom|Out of memory|Killed process", l, re.I)]
        (out / "dmesg_new_filtered.txt").write_text("\n".join(scrub(l) for l in keep) + "\n")
        (out / "END").write_text("%s rc=%s variant=%s\n" % (now(), rc, variant))
        shutil.rmtree(work, ignore_errors=True)
        return 0 if rc == 0 else 1
    finally:
        active("END kf3 offline clone check tag=%s variant=%s rc=%s" % (tag, variant, rc))
        fcntl.flock(fd, fcntl.LOCK_UN)


if __name__ == "__main__":
    sys.exit(main())

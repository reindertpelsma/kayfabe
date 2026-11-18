#!/usr/bin/env python3
"""Win11 base, GPU stage: NVIDIA 580.88 under kf3 (kayfabe QEMU + display broker), installed the way the gate does it, then verified,
then the repo's own per-boot repairs (power_fix, gpu_fix) applied once so they are baked, then an in-guest restart test.
Adapted from the image-B stage (b_gpu.py, MEASURED 2026-11-15) with: no profile env flags (only the RPC trace, a diagnostic), a bounded poll
instead of a fixed sleep, host readiness checks, a host census and dmesg delta, a registry readback, and a restart test.
Needs /root/prov/pc_gpu.lock (taken non-blocking for the whole run; refuses if held), the kayfabe QEMU, a TEST broker (headless, its own
socket under PROP_DIR/run: nothing appears on the owner's monitor), guest-driver 580.65.06 (state default), OVMF vars with the boot-GOP db
certificate, and the image shut down. Runs ON the pc as root. Usage: w_gpu.py [--disk PATH]   (default PROP_DIR/image.qcow2)
"""
import fcntl
import json
import os
import re
import shlex
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()
CK = load_check()
BROKER = os.environ.get("PROP_BROKER", "/workspace/bench/pc_vfio_20261116/src_6bfa2dbc2/broker/nvkvm-display-broker")
LOCK = "/root/prov/pc_gpu.lock"
KEY = D / "keys/id_ed25519"
OUT = D / "run/gpu"


def activelog(msg):
    with open("/root/prov/PC_ACTIVE.log", "a") as f:
        f.write("%s win11base %s\n" % (time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), msg))


def ps(st, script, timeout=300):
    return ssh_ps(st, KEY, script, timeout=timeout)


def scp(st, src, dst):
    return sh(["scp", "-q", "-r", "-i", str(KEY), "-P", str(st["ids"]["net"]["ssh_port"]), "-o", "StrictHostKeyChecking=no",
               "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", str(src), "%s@127.0.0.1:%s" % (st["guest"]["user"], dst)], timeout=1800)


def wait_ssh(st, timeout, alive=None):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if alive is not None and alive.poll() is not None:
            raise SystemExit("qemu gone while waiting for ssh (rc=%s)" % alive.returncode)
        try:
            p = sh(ssh_base(st, KEY) + ["echo", "up"], check=False, timeout=30)
            if p.returncode == 0 and "up" in p.stdout:
                return int(time.time() - t0)
        except subprocess.TimeoutExpired:
            pass
        time.sleep(8)
    raise SystemExit("ssh timeout")


def host_cuda_ready():
    """The host CUDA runtime must answer before kf3 starts (MEASURED 2026-11-17: cuInit 999 right after a vfio restore)."""
    probe = "import ctypes,sys; l=ctypes.CDLL('libcuda.so.1'); r=l.cuInit(0); print('cuInit',r); sys.exit(0 if r==0 else 1)"
    for i in range(8):
        p = sh(["python3", "-c", probe], check=False, timeout=60)
        (OUT / "host_cuda_ready.txt").open("a").write("try %d rc=%d %s\n" % (i, p.returncode, p.stdout.strip()))
        if p.returncode == 0:
            return True
        time.sleep(30)
    return False


CHECK_PS = r"""
$d=Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE*' -and $_.Class -eq 'Display' } | Select-Object -First 1
"PNP name='$($d.FriendlyName)' status=$($d.Status) problem=$($d.Problem) code=$((Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_ProblemCode).Data)"
nvidia-smi --query-gpu=name,uuid,driver_version --format=csv,noheader
"""

REG_PS = r"""
$ErrorActionPreference='SilentlyContinue'
$o=[ordered]@{}
$cls='HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$o.display_class_instances=@(Get-ChildItem $cls -Name | Where-Object {$_ -match '^\d{4}$'} | ForEach-Object { $p=Get-ItemProperty (Join-Path $cls $_); [ordered]@{key=$_;desc=$p.DriverDesc;provider=$p.ProviderName;ver=$p.DriverVersion;EnableGpuFirmware=$p.EnableGpuFirmware} })
$o.nvlddmkm_EnableGpuFirmware=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm').EnableGpuFirmware
$o.nvlddmkm_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\nvlddmkm').Start
$o.dwm_OverlayTestMode=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm').OverlayTestMode
$wl=Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
$o.winlogon=[ordered]@{AutoAdminLogon=$wl.AutoAdminLogon;DefaultUserName_set=[bool]$wl.DefaultUserName;DefaultPassword_set=[bool]$wl.DefaultPassword;DefaultDomainName=$wl.DefaultDomainName;AutoLogonCount_absent=($null -eq $wl.AutoLogonCount)}
$o.devicepasswordless=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\Device').DevicePasswordLessBuildVersion
$o.openssh_default_shell=(Get-ItemProperty 'HKLM:\SOFTWARE\OpenSSH').DefaultShell
$o.policies=[ordered]@{
 NoAutoUpdate=(Get-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU').NoAutoUpdate
 ExcludeWUDriversInQualityUpdate=(Get-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate').ExcludeWUDriversInQualityUpdate
 SearchOrderConfig=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\DriverSearching').SearchOrderConfig
 PreventDeviceEncryption=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\BitLocker').PreventDeviceEncryption
 EnableVirtualizationBasedSecurity=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard').EnableVirtualizationBasedSecurity
 HVCI_Enabled=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity').Enabled
 HiberbootEnabled=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power').HiberbootEnabled
 CrashDumpEnabled=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl').CrashDumpEnabled
 DisplayParameters=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CrashControl').DisplayParameters
 VerifiedAndReputablePolicyState=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy').VerifiedAndReputablePolicyState
 BypassNRO=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE').BypassNRO
 HwSchMode=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers').HwSchMode
 TdrLevel=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers').TdrLevel
 TdrDelay=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers').TdrDelay
}
$o.hibernate_file_present=(Test-Path C:\hiberfil.sys)
$o.sshd=[ordered]@{status=[string](Get-Service sshd).Status;start=[string](Get-Service sshd).StartType}
$o.kf_dir_present=(Test-Path C:\kf)
$o.bitlocker_c=((manage-bde -status C: | Select-String 'Conversion Status|Protection Status') -join ' ; ')
$o.windows_update_service=(Get-Service wuauserv).StartType.ToString()
$o | ConvertTo-Json -Compress -Depth 5
"""


QUIET_PS = r"""
$ok=0
for($i=0;$i -lt 60;$i++){
  $c=(Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'").PercentProcessorTime
  $ti=@(Get-Process TiWorker,TrustedInstaller,MoUsoCoreWorker,wuauclt,msiexec,setup,SetupHost -ErrorAction SilentlyContinue)
  if($c -lt 6 -and $ti.Count -eq 0){$ok++}else{$ok=0}
  if($ok -ge 4){ "QUIET after $i rounds"; exit 0 }
  Start-Sleep -Seconds 10
}
"NOT QUIET (last cpu $c; busy: $(($ti|ForEach-Object{$_.Name}) -join ','))"
"""


def wait_desktop(st):
    for _ in range(40):
        r = ps(st, r"(Get-Process explorer -IncludeUserName -ErrorAction SilentlyContinue | Select-Object -First 1).UserName", 60)
        if st["guest"]["user"].lower() in r.stdout.lower():
            return r.stdout.strip()
        time.sleep(8)
    return "no"


def poll_gpu(st, seconds, step=15):
    """Bounded poll for a Code 0 device and the seed UUID in nvidia-smi (replaces B's fixed 45 s sleep)."""
    t0 = time.time()
    last = ""
    while time.time() - t0 < seconds:
        chk = ps(st, CHECK_PS, 150)
        last = chk.stdout + chk.stderr
        if st["ids"]["gpu_uuid"] in chk.stdout and "code=0" in chk.stdout:
            return True, int(time.time() - t0), last
        time.sleep(step)
    return False, int(time.time() - t0), last


def main():
    disk = Path(sys.argv[sys.argv.index("--disk") + 1]) if "--disk" in sys.argv else D / "image.qcow2"
    st = json.loads((D / "state.json").read_text())
    PI.validate_state(st)
    assert PI.seed_matches(st)
    if qemu_pid():
        raise SystemExit("a win11-base qemu is running")
    need_free(8, "gpu stage")
    OUT.mkdir(exist_ok=True)
    lockfd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    try:
        fcntl.flock(lockfd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("GPU LOCK HELD by another session: not waiting, not touching the GPU")
    activelog("START gpu stage (kf3 install of NVIDIA 580.88 into the new Windows 11 base, headless test broker, guest offline)")
    procs = []
    result = {"started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    try:
        rc_, o_ = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], capture_output=True, text=True, timeout=60).returncode, ""
        if rc_ != 0:
            raise SystemExit("host nvidia-smi does not answer: not starting kf3")
        if not host_cuda_ready():
            raise SystemExit("host cuInit never ready: not starting kf3")
        dmesg0 = sh(["dmesg"]).stdout.count("\n")
        # --- host side: the driver tree from the pinned installer on tools.iso (read-only extraction)
        xdir = D / "run/nvx"
        if not (xdir / "Display.Driver/nv_dispi.inf").exists():
            shutil.rmtree(xdir, ignore_errors=True)
            xdir.mkdir()
            sh(["7z", "e", "-y", "-o" + str(xdir), str(D / "media/tools.iso"), PI.NVIDIA_EXE])
            assert PI.sha256_file(xdir / PI.NVIDIA_EXE) == PI.PINS[PI.NVIDIA_EXE]
            sh(["7z", "x", "-y", "-o" + str(xdir / "x"), str(xdir / PI.NVIDIA_EXE)])
            (xdir / "x/Display.Driver").rename(xdir / "Display.Driver")
            shutil.rmtree(xdir / "x", ignore_errors=True)
            (xdir / PI.NVIDIA_EXE).unlink()
        log("gpu: driver tree ready (%s)" % ", ".join(sorted(p.name for p in (xdir / "Display.Driver").iterdir())[:6]), "gpu.log")
        # --- processes: swtpm, test broker, kf3 qemu (root; kf3 opens /dev/nvidia*)
        start_swtpm(D)
        sock = D / "run/broker.sock"
        sock.unlink(missing_ok=True)
        fifo = D / "run/broker.fifo"
        fifo.unlink(missing_ok=True)
        os.mkfifo(fifo)
        fd = os.open(fifo, os.O_RDWR)
        bl_ = open(D / "run/broker.log", "w")
        bp = subprocess.Popen([BROKER, "--socket", str(sock), "--backend", "test", "--persist", "--verbose", "--size", "1920x1080"],
                              stdin=fd, stdout=bl_, stderr=subprocess.STDOUT)
        procs.append(bp)
        for _ in range(50):
            if sock.exists():
                break
            time.sleep(0.2)
        assert sock.exists(), "broker made no socket"
        argv = PI.build_argv(st, "install", str(disk), str(D), qemu=KF_QEMU, broker_sock=str(sock),
                             gop_efi=str(D / "firmware/kf-gop.signed.efi"), uid=0, display="broker")
        argv = [a.replace("restrict=off", "restrict=on") for a in argv]
        argv = ["propimage-w11-install" if a == "propimage-install" else a for a in argv]
        assert "restrict=on" in " ".join(argv)
        env = dict(os.environ)
        env_added = {}
        for line in (D / "bin/kf3_flags.sample.env").read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            assert re.fullmatch(r"KF3_[A-Z0-9_]+=\d+", line), line
            k, v = line.split("=")
            env[k] = v
            env_added[k] = v
        env["KF3_RPC_TRACE"] = "1"
        env_added["KF3_RPC_TRACE"] = "1"
        result["kf3_env"] = env_added
        (D / "run/last_cmd_gpu.txt").write_text(shlex.join(argv) + "\n")
        (OUT / "kf3_env.json").write_text(json.dumps(env_added, indent=1) + "\n")
        ql = open(D / "run/qemu_gpu.log", "wb")
        qp = subprocess.Popen(["nice", "-n", "10"] + argv, env=env, stdout=ql, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        procs.append(qp)
        log("gpu: qemu started pid %d; waiting for ssh" % qp.pid, "gpu.log")
        time.sleep(3)
        if qp.poll() is not None:
            raise SystemExit("qemu exited rc=%s within 3 s; log: %s ; swtpm log tail: %s" % (qp.returncode, (D / "run/qemu_gpu.log").read_text()[-800:],
                             (D / "run/swtpm.log").read_text()[-400:]))
        result["ssh_up_s"] = wait_ssh(st, 900, qp)
        log("gpu: ssh up after %d s" % result["ssh_up_s"], "gpu.log")
        # --- driver install, device disabled (INF route: EnableGpuFirmware is written before nvlddmkm's first start)
        ps(st, r"New-Item -Force -ItemType Directory C:\kf\nv | Out-Null")
        scp(st, D / "run/nvx/Display.Driver", "C:/kf/nv/")
        for f in ("gsp_on.ps1", "desktop_prep.ps1", "gate_enable.ps1"):
            scp(st, D / "bin" / f, "C:/kf/" + f)
        t_inst = time.time()
        r = ps(st, r"powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\gsp_on.ps1 -Inf C:\kf\nv\Display.Driver\nv_dispi.inf -NoEnable", 1200)
        result["driver_install_s"] = int(time.time() - t_inst)
        (OUT / "gsp_on.txt").write_text(r.stdout + r.stderr)
        log("gpu: gsp_on rc=%d (%d s): %s" % (r.returncode, result["driver_install_s"], " | ".join(l for l in r.stdout.splitlines() if "readback" in l or "WARN" in l or "REFUSED" in l)), "gpu.log")
        # autologon (password on stdin only; never logged)
        pw = PI.derive_password(st["seed"])
        p = subprocess.run(ssh_base(st, KEY) + ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", r"C:\kf\desktop_prep.ps1",
                                               "-User", st["guest"]["user"], "-PasswordFromStdin"], input=pw + "\n", text=True, capture_output=True, timeout=120)
        (OUT / "desktop_prep.txt").write_text(p.stdout)
        log("gpu: desktop_prep: %s" % (p.stdout.strip().splitlines() or ["(no output)"])[-1], "gpu.log")
        # reboot, wait for the signed-in desktop, then enable ONCE
        ps(st, "Restart-Computer -Force", 60)
        time.sleep(45)
        log("gpu: ssh back after %d s" % wait_ssh(st, 900, qp), "gpu.log")
        desk = wait_desktop(st)
        log("gpu: signed-in desktop: %s" % desk, "gpu.log")
        if desk == "no":
            raise SystemExit("no signed-in desktop after autologon; NOT enabling the device (Code 43 risk)")
        time.sleep(20)
        r = ps(st, r"powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_enable.ps1", 120)
        (OUT / "gate_enable.txt").write_text(r.stdout + r.stderr)
        log("gpu: %s" % r.stdout.strip(), "gpu.log")
        ok, waited, last = poll_gpu(st, 300)
        (OUT / "gpu_check.txt").write_text(last)
        result["first_enable_ok"], result["first_enable_poll_s"] = ok, waited
        log("gpu: check after %d s: %s" % (waited, last.strip().replace("\n", " | ")), "gpu.log")
        log("gpu: RESULT %s (Code 0 and seed GPU UUID)" % ("OK" if ok else "FAILED"), "gpu.log")
        if ok:
            # the repo's own per-boot repairs, once, so they are baked and the sealed image needs no change on its first clone boot
            key = D / "run/id_kf"
            key.write_text(st["guest"]["ssh_private_key"])
            key.chmod(0o600)
            g = CK.Guest(st, str(key), st["ids"]["net"]["ssh_port"])
            power = CK.power_fix(g)
            (OUT / "power1.json").write_text(json.dumps(power, indent=2))
            result["power1_ok"] = bool(power["ok"])
            log("gpu: power_fix #1 %s %s" % ("OK" if power["ok"] else "NOT OK", "; ".join(power["errors"])[:300]), "gpu.log")
            fixed, detail = CK.gpu_fix(g, st["ids"]["gpu_uuid"])
            (OUT / "gpu_fix1.txt").write_text(detail.replace(st["ids"]["gpu_uuid"], "<seed-uuid>"))
            result["gpu_fix1_ok"] = bool(fixed)
            log("gpu: gpu_fix #1 %s" % fixed, "gpu.log")
            reg = g.ps(REG_PS, 120)
            (OUT / "registry_1.json").write_text(reg.stdout)
            # full nvidia-smi, uuid and serial-like values stripped on the host before anything leaves the pc
            smi = ps(st, "nvidia-smi; nvidia-smi -q | Select-Object -First 120", 150)
            (OUT / "nvidia_smi_1.txt").write_text(re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", smi.stdout))
            ps(st, r"Remove-Item -Recurse -Force C:\kf -ErrorAction SilentlyContinue; Test-Path C:\kf", 60)
            # restart test: in-guest restart inside the same QEMU (kf3 reset path), the GPU must come back by itself or after gpu_fix
            log("gpu: restart test (in-guest Restart-Computer inside the same kf3 QEMU)", "gpu.log")
            ps(st, "Restart-Computer -Force", 60)
            time.sleep(45)
            result["restart_ssh_back_s"] = wait_ssh(st, 900, qp)
            desk2 = wait_desktop(st)
            ok2, waited2, last2 = poll_gpu(st, 240)
            (OUT / "gpu_check_after_restart.txt").write_text(last2)
            result["restart_gpu_ok"], result["restart_poll_s"], result["restart_desktop"] = ok2, waited2, desk2 != "no"
            log("gpu: after restart: desktop=%s gpu_ok=%s (%d s): %s" % (desk2 != "no", ok2, waited2, last2.strip().replace("\n", " | ")), "gpu.log")
            if not ok2:
                fixed2, detail2 = CK.gpu_fix(g, st["ids"]["gpu_uuid"])
                (OUT / "gpu_fix2.txt").write_text(detail2.replace(st["ids"]["gpu_uuid"], "<seed-uuid>"))
                result["restart_needed_gpu_fix"], result["gpu_fix2_ok"] = True, bool(fixed2)
                log("gpu: restart needed gpu_fix: %s" % fixed2, "gpu.log")
            power2 = CK.power_fix(g)
            (OUT / "power2.json").write_text(json.dumps(power2, indent=2))
            result["power2_ok"] = bool(power2["ok"])
            log("gpu: power_fix #2 %s %s" % ("OK" if power2["ok"] else "NOT OK", "; ".join(power2["errors"])[:300]), "gpu.log")
            reg2 = g.ps(REG_PS, 120)
            (OUT / "registry_2.json").write_text(reg2.stdout)
            smi2 = ps(st, "nvidia-smi", 150)
            (OUT / "nvidia_smi_2.txt").write_text(re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", smi2.stdout))
            quiet = g.ps(QUIET_PS, 900)      # Guest.ps copies a multi-line script to a file (stdin parsing of blocks printed nothing on image B)
            result["quiesce"] = (quiet.stdout.strip() or quiet.stderr.strip())[:200]
            log("gpu: quiesce: %s" % result["quiesce"], "gpu.log")
            ok_final = ok2 or result.get("gpu_fix2_ok")
            if ok_final:
                shutil.copyfile(D / "state.json", D / "run/state.before_gpu_record.json")
                guest = PI.run_guest_probe(st, str(KEY))
                PI.fill_from_guest(st, guest)
                (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
                (OUT / "probe_gpu.json").write_text(json.dumps(guest, indent=1))
                log("gpu: recorded; machine_guid/ek unchanged check: see probe", "gpu.log")
            key.unlink(missing_ok=True)
            ok = bool(ok_final)
        ps(st, "Stop-Computer -Force", 60)
        qp.wait(timeout=360)
        # host-side evidence: census of the kf3 log, dmesg delta (filtered, identifiers stripped)
        sys.path.insert(0, str(D / "bin"))
        import kf3_census
        lines = [l.decode("utf-8", "replace") for l in open(D / "run/qemu_gpu.log", "rb")]
        c = kf3_census.census(lines)
        (OUT / "census.json").write_text(json.dumps(c, indent=1))
        tail = [kf3_census.clean(l.rstrip("\n")) for l in lines[-200:]]
        (OUT / "qemu_log_tail.txt").write_text("\n".join(tail) + "\n")
        d = sh(["dmesg"]).stdout.splitlines()[dmesg0:]
        keep = [l for l in d if re.search(r"NVRM|Xid|kf3|nvidia|DMAR|AMD-Vi|IOMMU|AER|BUG:|Oops|oom|Out of memory|Killed process|kvm", l, re.I)]
        (OUT / "dmesg_new_filtered.txt").write_text("\n".join(re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", l) for l in keep) + "\n")
        result["census_counts"], result["xid"], result["dmesg_new_lines"] = c["counts"], c["xid"], len(keep)
        windows_lines = [kf3_census.clean(l.rstrip("\n")) for l in lines if re.search(r"WINDOWS PROFILE|FABRICATED answer|caps page derived|scheduled_at_birth|gr_tier", l)]
        (OUT / "windows_profile_lines.txt").write_text("\n".join(windows_lines[:80]) + "\n")
        result["windows_profile_line_count"] = len(windows_lines)
        result["ok"] = bool(ok)
        result["ended"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        (OUT / "result.json").write_text(json.dumps(result, indent=1) + "\n")
        log("gpu: STAGE RESULT %s" % json.dumps(result), "gpu.log")
        return 0 if ok else 1
    finally:
        for p in reversed(procs):
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    p.kill()
        shutil.rmtree(D / "run/nvx", ignore_errors=True)
        (D / "run/id_kf").unlink(missing_ok=True)
        activelog("END gpu stage (see PROP_DIR/run/gpu.log)")
        fcntl.flock(lockfd, fcntl.LOCK_UN)


if __name__ == "__main__":
    sys.exit(main())

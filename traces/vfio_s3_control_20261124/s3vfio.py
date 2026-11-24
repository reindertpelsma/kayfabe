#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""s3vfio.py -- the VFIO control of the Windows S3 wake storm on the pc (STATUS: RESEARCH, 2026-11-24; evidence tool of
traces/vfio_s3_control_20261124/, run by hand as root on the pc; not a gate).

  s3vfio.py prep    --base SEALED_DIR --vmroot DIR --out DIR --qemu BIN
  s3vfio.py session --base SEALED_DIR --vmroot DIR --out DIR --plan ARM:QEMU:CYCLES[,...] [--stop-new-boots HH:MM]

prep: one boot WITHOUT a GPU (no bind) of a child of the sealed image: checks ssh and the signed-in session, turns on
autologon for the image's own account if it is off, hibernation off, standby/monitor timeouts 0, clean shutdown. The
result, prep.qcow2, is the backing file of every run boot.
session: takes /root/prov/pc_gpu.lock, writes PC_ACTIVE.log lines, records the host state, binds the GPU with the safe bind
of the 2026-11-18 session (pcsession.py beside this file, unchanged), runs the arms of PREREGISTRATION.md in the order of
--plan, and ALWAYS restores (pcsession.restore_vfio: nvidia modules, audio function, persistenced, gdm, nvidia-smi, uvm
settle) and records the host state again. Per cycle it reuses the trace and judging helpers of the 2026-11-23 probe
(traces/win11_s3_wake_20261123/s3probe.py). The guest's and the host's output is data, only parsed.
"""
import argparse
import fcntl
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "scripts" / "bench" / "windows"))
sys.path.insert(0, str(REPO / "traces" / "win11_s3_wake_20261123"))
sys.path.insert(0, str(HERE))
import propimage as PI  # noqa: E402
import qmp as qmpmod  # noqa: E402
import s3probe as SP  # noqa: E402

LOCK = "/root/prov/pc_gpu.lock"
ACTIVE = Path("/root/prov/PC_ACTIVE.log")
GPU = "0000:01:00.0"
AUDIO = "0000:01:00.1"
ARM_CPU = {
    "A": "host,-vmx",
    "B": "host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash",
}
ARM_CPU["BP"] = ARM_CPU["B"]
ARM_CPU["B0"] = ARM_CPU["B"]
EVENT_IDS = (41, 1001, 6008, 1074, 153, 14, 4101, 42, 1, 107)
OUT = Path(".")


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def say(msg):
    SP.say(msg)


def active(msg):
    with ACTIVE.open("a") as f:
        f.write("%s vfio-s3 %s\n" % (now(), msg))


def sh(cmd, timeout=120):
    r = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True, timeout=timeout)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def scrub(text):
    text = re.sub(r"GPU-[0-9a-fA-F-]{20,}", "GPU-<uuid>", text)
    text = re.sub(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b", "<uuid>", text)
    text = re.sub(r"\b(\d{1,3}\.){3}\d{1,3}\b", "<ip>", text)
    text = re.sub(r"(?i)\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b", "<mac>", text)
    return re.sub(r"(?i)(serial[=: ]+)[^,\s]+", r"\1<serial>", text)


class Vm:
    """One boot: a throwaway child overlay, swtpm, QEMU. Duck-types what s3probe's helpers need (qpid, qmp, hmp)."""

    def __init__(self, state, vmroot, name, backing, qemu, cpu, gpu, disk=None):
        self.state, self.name, self.qemu, self.cpu, self.gpu = state, name, qemu, cpu, gpu
        self.work = Path(vmroot) / name
        self.backing = backing
        self.disk = Path(disk) if disk else self.work / "disk.qcow2"
        self.proc = None

    def setup(self):
        self.work.mkdir(parents=True)
        PI.restore_files(self.state, self.work)
        if self.backing:
            rc, o = sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(self.backing), "-F", "qcow2", str(self.disk)])
            if rc:
                raise RuntimeError("qemu-img create: " + o[-300:])
        key = self.work / "key"
        key.write_text(self.state["guest"]["ssh_private_key"])
        key.chmod(0o600)

    def argv(self):
        a = PI.build_argv(self.state, PI.PLAIN, self.disk, self.work, qemu=self.qemu, display="off", net="off")
        i = a.index("-cpu")
        a[i + 1] = self.cpu
        i = next(n for n, v in enumerate(a) if v == "-device" and a[n + 1].startswith("kf3-gpu"))
        del a[i:i + 2]
        if self.gpu:
            a[i:i] = ["-device", "vfio-pci,host=%s,multifunction=on,addr=0x4.0" % GPU,
                      "-device", "vfio-pci,host=%s,addr=0x4.1" % AUDIO]
        a += ["-global", "ICH9-LPC.disable_s3=0", "-pidfile", str(self.work / "qemu.pid")]
        assert not any("kf3" in x for x in a), "kf3 device left in argv"
        return a

    def start(self):
        rc, o = sh(PI.swtpm_argv(self.work), 30)
        if rc:
            raise RuntimeError("swtpm: " + o[-300:])
        a = self.argv()
        (self.work / "argv.txt").write_text(" ".join(a) + "\n")
        log = open(self.work / "qemu.log", "w")
        self.proc = subprocess.Popen(a, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
        return a

    def qpid(self):
        return self.proc.pid if self.proc and self.proc.poll() is None else None

    def guest(self, ps, timeout=60):
        argv = PI.ssh_argv(self.state, self.work / "key")
        try:
            r = subprocess.run(["timeout", str(timeout)] + argv, input=ps, capture_output=True, text=True, timeout=timeout + 10)
            return r.returncode, (r.stdout or "").replace("\r", "")
        except subprocess.TimeoutExpired:
            return 124, ""

    def qmp(self, name, args=None, tries=5):
        last = None
        for _ in range(tries):
            try:
                s, f = qmpmod.qmp_open(str(self.work / "qmp.sock"), timeout=10)
                try:
                    r = qmpmod.qmp_cmd(f, name, args)
                    if "error" in r:
                        raise RuntimeError(r["error"])
                    return r.get("return", r)
                finally:
                    s.close()
            except Exception as e:
                last = e
                time.sleep(0.7)
        raise RuntimeError("QMP %s: %s" % (name, last))

    def hmp(self, line):
        return self.qmp("human-monitor-command", {"command-line": line})

    def status(self):
        try:
            return self.qmp("query-status").get("status", "?")
        except Exception as e:
            return "qmp-error:%s" % e

    def wait_up(self, limit=900):
        t = time.time()
        while time.time() - t < limit:
            if self.proc.poll() is not None:
                return False
            rc, o = self.guest('"KF_UP"', 20)
            if "KF_UP" in o:
                return True
            time.sleep(10)
        return False

    def stop(self, clean=True):
        """Guest shutdown when it answers, else QMP quit, else SIGKILL. Then swtpm ends by itself (--terminate)."""
        if self.qpid() is None:
            return "not running"
        how = []
        if clean:
            rc, _ = self.guest("Stop-Computer -Force", 30)
            how.append("guest Stop-Computer rc=%s" % rc)
            try:
                self.proc.wait(240)
                return "; ".join(how + ["exited %s" % self.proc.returncode])
            except subprocess.TimeoutExpired:
                pass
        try:
            self.qmp("quit", tries=2)
            how.append("qmp quit")
        except Exception as e:
            how.append("qmp quit failed %s" % e)
        try:
            self.proc.wait(60)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            how.append("SIGKILL")
            self.proc.wait(60)
        return "; ".join(how + ["exited %s" % self.proc.returncode])


def guest_facts(vm):
    ps = ('$o=Get-CimInstance Win32_OperatingSystem; "BOOT " + $o.LastBootUpTime.ToString("o"); '
          '"NOW " + (Get-Date).ToString("o"); '
          '$e=Get-Process explorer -EA 0 | ? { $_.SessionId -gt 0 }; "EXPLORER " + [int]($e -ne $null); '
          '"QUSER " + ((quser 2>$null) -join " | ")')
    rc, o = vm.guest(ps, 60)
    f = {"rc": rc}
    for ln in o.splitlines():
        k, _, v = ln.partition(" ")
        if k in ("BOOT", "NOW", "EXPLORER", "QUSER"):
            f[k.lower()] = v.strip()
    return f


def guest_events(vm, since):
    ids = ",".join(str(i) for i in EVENT_IDS)
    ps = ('Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=[datetime]::Parse("SINCE")} -EA 0 | '
          '? { $_.Id -in IDS } | sort TimeCreated | % { $m = ($_.Message + "") -replace "\\s+"," "; '
          '"$($_.TimeCreated.ToString("o")) $($_.Id) $($_.ProviderName) " + $m.Substring(0, [Math]::Min(120, $m.Length)) }')
    ps = ps.replace("SINCE", since).replace("IDS", ids)
    rc, o = vm.guest(ps, 120)
    return [scrub(x) for x in o.strip().splitlines() if x.strip()]


def storm_signature(trace_summary, rates_late):
    distinct = trace_summary.get("stimer_distinct", {})
    top = max(distinct.values()) if distinct else 0
    wraps = any(s.get("wraps_x100") for s in trace_summary.get("stimer_first", []))
    exits = (rates_late or {}).get("exits", 0)
    return {"top_rearm": top, "wraps_x100": wraps, "exits_per_s_late": exits,
            "signature": bool((wraps and top >= 10000) or exits > 100000)}


def cycle(vm, n, out, label):
    f0 = guest_facts(vm)
    boot0 = f0.get("boot", "")
    pre_text = SP.read_trace()
    (out / ("c%d_boot_trace_tail.txt" % n)).write_text(pre_text[-200000:])
    pre = SP.summarize(SP.parse_trace(pre_text), None)
    gpa = SP.tsc_page_gpa(pre["hv_writes_before_wake"])
    say("%s c%d boot0=%s explorer=%s hv_writes_before=%d tsc_page_gpa=%s" % (label, n, boot0, f0.get("explorer"),
        len(pre["hv_writes_before_wake"]), hex(gpa) if gpa else None))
    page = {}
    if gpa:
        page["before_suspend"] = vm.hmp("xp /4gx %#x" % gpa)
    marker = f0.get("now", "")
    SP.wput("tracing_on", "0")
    SP.wput("trace", "")
    SP.wput("tracing_on", "1")
    user = vm.state["guest"]["user"]
    rc, o = vm.guest('schtasks /create /tn kfs3 /tr "rundll32.exe powrprof.dll,SetSuspendState 0,1,0" /sc once /st 23:59 '
                     '/ru %s /it /f | Out-Null; schtasks /run /tn kfs3; "KF_SCHED"' % user, 60)
    say("%s c%d suspend requested rc=%s %s" % (label, n, rc, o.strip()[-60:]))
    st, t = "", time.time()
    while time.time() - t < 120:
        st = vm.status()
        if st == "suspended":
            break
        time.sleep(1)
    res = {"label": label, "cycle": n, "boot0": boot0, "explorer_before": f0.get("explorer"), "quser_before": f0.get("quser"),
           "suspended": st == "suspended", "suspend_wait_s": round(time.time() - t)}
    say("%s c%d status=%s after %ds" % (label, n, st, res["suspend_wait_s"]))
    if st != "suspended":
        _, req = vm.guest("powercfg /requests; powercfg /a", 90)
        (out / ("c%d_no_suspend_diag.txt" % n)).write_text(scrub(req))
        res.update(verdict="FAIL-OTHER", reasons=["never suspended"], guest_alive=True)
        return res
    if gpa:
        page["while_suspended"] = vm.hmp("xp /4gx %#x" % gpa)
    SP.tr_events(SP.WAKE_EVENTS, True)
    SP.wput("trace_marker", "KF_S3_WAKE")
    host_wake = time.time()
    res["wake_utc"] = now()
    vm.qmp("system_wakeup")
    time.sleep(4)
    SP.tr_events(SP.WAKE_EVENTS, False)
    res["rates_wake+4s"] = SP.rates(vm.qpid(), 2.0)
    if gpa:
        page["after_wake"] = vm.hmp("xp /4gx %#x" % gpa)
    res["tsc_page"] = page
    res["tsc_page_unchanged"] = bool(gpa) and page.get("before_suspend") == page.get("after_wake")
    text = SP.read_trace()
    (out / ("c%d_wake_trace_head.txt" % n)).write_text("\n".join(text.splitlines()[:30000]) + "\n")
    evs = SP.parse_trace(text)
    wake_ts = next((ts for ts, ev, body, _ in evs if ev in ("print", "tracing_mark_write") and "KF_S3_WAKE" in body), None)
    res["trace"] = SP.summarize(evs, wake_ts)
    say("%s c%d woke; status=%s rates=%s stimer_first=%s" % (label, n, vm.status(), res["rates_wake+4s"],
        res["trace"]["stimer_first"][:2]))
    ok, back, late = False, None, None
    while time.time() - host_wake < 150:
        rc, o = vm.guest('"KF_BACK"', 20)
        if "KF_BACK" in o:
            ok, back = True, round(time.time() - host_wake)
            break
        if time.time() - host_wake > 60 and late is None:
            late = SP.rates(vm.qpid(), 2.0)
            SP.wedge_sample(vm, out, n)
        time.sleep(5)
    res.update(ssh_back=ok, back_after_s=back, rates_late=late, status_after=vm.status())
    res["storm"] = storm_signature(res["trace"], late)
    if not ok:
        res["guest_alive"] = False
        res["verdict"] = "STORM" if res["storm"]["signature"] else "FAIL-OTHER"
        res["reasons"] = ["no ssh within 150 s"]
        return res
    say("%s c%d ssh back after %ds; waiting 150 s for a post-wake TDR" % (label, n, back))
    time.sleep(150)
    f1 = guest_facts(vm)
    _, smi = vm.guest("nvidia-smi --query-gpu=name,driver_version --format=csv,noheader", 60)
    evl = guest_events(vm, marker)
    res.update(boot1=f1.get("boot", ""), explorer_after=f1.get("explorer"), nvidia_smi=scrub(smi.strip())[-200:], events=evl,
               guest_alive=True)
    res["event153_nvlddmkm"] = sum(1 for e in evl if re.search(r" 153 nvlddmkm\b", e))
    reasons = []
    if res["boot1"] != boot0:
        reasons.append("new boot (LastBootUpTime changed)")
    if not re.search(r"RTX 4070", smi):
        reasons.append("nvidia-smi does not name the RTX 4070")
    if f1.get("explorer") != "1":
        reasons.append("no explorer in an interactive session")
    if any(re.search(r" (41|1001|6008) ", " " + e + " ") for e in evl):
        reasons.append("event 41/1001/6008")
    res["verdict"] = "CLEAN" if not reasons else "FAIL-OTHER"
    res["reasons"] = reasons
    return res


def host_state(tag):
    cmds = [
        "date -u",
        "cat /sys/devices/system/clocksource/clocksource0/current_clocksource",
        "dmesg | grep -iE 'tsc.*unstable|clocksource.*(unstable|switched)' | tail -5",
        "nvidia-smi --query-gpu=name,driver_version --format=csv,noheader",
        "lspci -nnk -s 01:00",
        "for c in /sys/class/drm/card*-*; do echo \"$c $(cat $c/status)\"; done",
        "for c in /sys/class/drm/card[0-9]; do echo \"$c -> $(readlink -f $c/device)\"; done",
        "systemctl is-active gdm nvidia-persistenced",
        "lsmod | grep -E '^(nvidia|vfio)'",
        "cat /sys/module/nvidia_uvm/initstate 2>/dev/null; cat /sys/module/nvidia_uvm/refcnt 2>/dev/null",
        "loginctl list-sessions --no-legend",
    ]
    lines = ["# host state %s %s" % (tag, now())]
    for c in cmds:
        rc, o = sh(["bash", "-c", c], 60)
        lines.append("$ %s\n%s" % (c, o.rstrip()))
    return scrub("\n".join(lines) + "\n")


def run_arm(state, a, arm, qemu, cycles, deadline, results):
    """Boots until `cycles` cycles have run (a storm ends its boot), or the boot deadline passes."""
    done, boot_n = 0, 0
    while done < cycles and boot_n < cycles:
        if deadline and time.time() > deadline:
            say("ARM %s: stop-new-boots time reached, %d of %d cycles run" % (arm, done, cycles))
            break
        boot_n += 1
        label = "%s_b%d" % (arm, boot_n)
        out = OUT / label
        out.mkdir(parents=True)
        vm = Vm(state, a.vmroot, label, Path(a.vmroot) / "prep.qcow2", qemu, ARM_CPU[arm], gpu=True)
        vm.setup()
        SP.arm_boot_trace()
        dm0 = sh("dmesg | wc -l")[1].strip()
        argv = vm.start()
        (out / "argv_scrubbed.txt").write_text(scrub(" ".join(argv)) + "\n")
        active("BOOT %s qemu=%s cpu=%s" % (label, qemu, ARM_CPU[arm]))
        up = vm.wait_up()
        say("%s guest up=%s" % (label, up))
        if not up:
            (out / "boot_failed.txt").write_text("guest did not answer ssh in 900 s; qemu rc=%s\n" % vm.proc.poll())
            say("%s %s" % (label, vm.stop(clean=False)))
            results.append({"label": label, "cycle": 0, "verdict": "NOT-RUN", "reasons": ["boot failed"]})
            continue
        time.sleep(90)
        _, pa = vm.guest("powercfg /a", 60)
        (out / "powercfg_a.txt").write_text(scrub(pa))
        storm = False
        while done < cycles:
            done += 1
            r = cycle(vm, done, out, label)
            r["arm"], r["qemu"] = arm, qemu
            results.append(r)
            (out / ("c%d_result.json" % done)).write_text(json.dumps(r, indent=1, default=str))
            say("S3_CYCLE arm=%s boot=%s n=%d verdict=%s 153=%s reasons=%s" % (arm, label, done, r["verdict"],
                r.get("event153_nvlddmkm"), r.get("reasons")))
            if not r.get("guest_alive", True) or r["verdict"] != "CLEAN":
                storm = True
                break
            time.sleep(30)
        stop = vm.stop(clean=not storm)
        say("%s stop: %s" % (label, stop))
        d = sh("dmesg | tail -n +%d" % (int(dm0 or 0) + 1))[1].splitlines()
        keep = [ln for ln in d if re.search(r"vfio|AER|NVRM|Xid|kvm|DMAR|AMD-Vi|BUG:|Oops|hung", ln, re.I)]
        (out / "dmesg_filtered.txt").write_text(scrub("\n".join(keep[-400:])) + "\n")
        qlog = (vm.work / "qemu.log").read_text(errors="replace")
        (out / "qemu_log.txt").write_text(scrub(qlog[-20000:]))
    return done


def cmd_prep(a):
    global OUT
    OUT = Path(a.out)
    OUT.mkdir(parents=True, exist_ok=True)
    SP.OUT = OUT
    state = json.loads((Path(a.base) / "state.json").read_text())
    prep = Path(a.vmroot) / "prep.qcow2"
    if not prep.exists():
        rc, o = sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(Path(a.base) / "image.qcow2"), "-F", "qcow2", str(prep)])
        if rc:
            raise SystemExit("qemu-img create prep: " + o)
    vm = Vm(state, a.vmroot, "prep_boot_%d" % int(time.time()), None, a.qemu, ARM_CPU["A"], gpu=False, disk=prep)
    vm.setup()
    vm.start()
    say("PREP boot started (no GPU) qemu=%s" % a.qemu)
    up = vm.wait_up()
    say("PREP up=%s" % up)
    if not up:
        say("PREP stop: %s" % vm.stop(clean=False))
        raise SystemExit(2)
    time.sleep(60)
    f = guest_facts(vm)
    say("PREP facts explorer=%s quser=%s" % (f.get("explorer"), f.get("quser")))
    user = state["guest"]["user"]
    ps = ("$wl='HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'; $g=Get-ItemProperty $wl; "
          "\"AUTOLOGON \" + $g.AutoAdminLogon + \" \" + $g.DefaultUserName + \" pw_set=\" + [int]($g.DefaultPassword -ne $null)")
    _, o = vm.guest(ps, 60)
    say("PREP before: " + o.strip())
    if f.get("explorer") != "1" or "AUTOLOGON 1 %s pw_set=1" % user not in o:
        pw = state["guest"]["password"].replace("'", "''")
        ps = ("$wl='HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'; "
              "Set-ItemProperty $wl AutoAdminLogon '1'; Set-ItemProperty $wl DefaultUserName '%s'; "
              "Set-ItemProperty $wl DefaultPassword '%s'; Set-ItemProperty $wl DefaultDomainName '.'; "
              "Remove-ItemProperty $wl AutoLogonCount -EA 0; "
              "$pl='HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\Device'; New-Item $pl -Force | Out-Null; "
              "Set-ItemProperty $pl DevicePasswordLessBuildVersion 0 -Type DWord; \"AUTOLOGON_SET\"" % (user, pw))
        _, o = vm.guest(ps, 60)
        say("PREP autologon: " + o.strip()[-40:])
    _, o = vm.guest("powercfg /hibernate off; powercfg /change standby-timeout-ac 0; powercfg /change monitor-timeout-ac 0; "
                    "powercfg /change hibernate-timeout-ac 0; \"POWERCFG_DONE\"", 60)
    say("PREP " + o.strip()[-40:])
    say("PREP stop: %s" % vm.stop(clean=True))
    # second boot: the session must come up by itself
    vm2 = Vm(state, a.vmroot, "prep_check_%d" % int(time.time()), prep, a.qemu, ARM_CPU["B"], gpu=False)
    vm2.setup()
    vm2.start()
    up = vm2.wait_up()
    time.sleep(60)
    f = guest_facts(vm2) if up else {}
    say("PREP_CHECK up=%s explorer=%s quser=%s boot=%s" % (up, f.get("explorer"), f.get("quser"), f.get("boot")))
    say("PREP_CHECK stop: %s" % vm2.stop(clean=True))
    say("PREP_DONE explorer=%s" % f.get("explorer"))


def cmd_session(a):
    global OUT
    OUT = Path(a.out)
    OUT.mkdir(parents=True, exist_ok=True)
    SP.OUT = OUT
    import pcsession as PS
    state = json.loads((Path(a.base) / "state.json").read_text())
    plan = []
    for item in a.plan.split(","):
        arm, q, n = item.split(":")
        assert arm in ARM_CPU and os.access(q, os.X_OK), item
        plan.append((arm, q, int(n)))
    deadline = None
    if a.stop_new_boots:
        hh, mm = (int(x) for x in a.stop_new_boots.split(":"))
        t = time.gmtime()
        deadline = time.mktime((t.tm_year, t.tm_mon, t.tm_mday, hh, mm, 0, 0, 0, 0)) - time.timezone
    fd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)        # a held lock is an error here: one pc job at a time
    bound = {"v": False}

    def term(sig, frm):
        raise SystemExit("signal %d" % sig)
    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, term)
    active("START VFIO S3 control (plan %s): GPU 01:00.0/.1 to vfio-pci, desktop dark until the restore" % a.plan)
    results = []
    try:
        (OUT / "host_before.txt").write_text(host_state("before"))
        try:
            ok, log = PS.bind_vfio()
        except PS.Abort as e:
            (OUT / "bind.txt").write_text(scrub(str(e)))
            active("ABORT bind: %s" % str(e).replace("\n", " | ")[-300:])
            bound["v"] = PS.recover_after_bind_abort(e, active_fn=active)
            return 9
        (OUT / "bind.txt").write_text(scrub(log))
        bound["v"] = True
        if not ok:
            active("bind incomplete, restoring")
            return 4
        active("BOUND: both functions on vfio-pci")
        say("BIND_OK")
        for arm, q, n in plan:
            run_arm(state, a, arm, q, n, deadline, results)
        return 0
    finally:
        (OUT / "results.json").write_text(json.dumps(results, indent=1, default=str))
        if bound["v"]:
            try:
                rlog = PS.restore_vfio()
            except PS.Abort as e:
                rlog = "RESTORE ABORT\n" + str(e)
                active("RESTORE ABORT: %s" % str(e).replace("\n", " | ")[-300:])
            (OUT / "restore.txt").write_text(scrub(rlog))
        (OUT / "host_after.txt").write_text(host_state("after"))
        rc, o = sh(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], 60)
        active("END VFIO S3 control: %d cycle records; host GPU: %s" % (len(results), o.strip()[:60] if rc == 0 else "rc=%s" % rc))
        say("SESSION_EXIT")
        os.close(fd)


def main():
    p = argparse.ArgumentParser()
    sp = p.add_subparsers(dest="cmd", required=True)
    for name in ("prep", "session"):
        r = sp.add_parser(name)
        r.add_argument("--base", required=True)
        r.add_argument("--vmroot", required=True)
        r.add_argument("--out", required=True)
        if name == "prep":
            r.add_argument("--qemu", required=True)
        else:
            r.add_argument("--plan", required=True)
            r.add_argument("--stop-new-boots", default="")
    a = p.parse_args()
    sys.exit(cmd_prep(a) if a.cmd == "prep" else cmd_session(a))


if __name__ == "__main__":
    main()
